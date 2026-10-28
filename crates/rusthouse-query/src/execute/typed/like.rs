use super::*;

pub(super) fn literal_bytes(tokens: &[LikeToken]) -> Option<Vec<u8>> {
    tokens
        .iter()
        .map(|token| match token {
            LikeToken::Byte(byte) => Some(*byte),
            LikeToken::Any | LikeToken::One => None,
        })
        .collect()
}

pub(super) fn compile_like(pattern: &str) -> Result<LikePattern, ExecutionError> {
    let mut tokens = Vec::new();
    let mut bytes = pattern.bytes();
    while let Some(byte) = bytes.next() {
        match byte {
            b'%' => {
                if !matches!(tokens.last(), Some(LikeToken::Any)) {
                    tokens.push(LikeToken::Any);
                }
            }
            b'_' => tokens.push(LikeToken::One),
            b'\\' => {
                let escaped = bytes.next().ok_or_else(|| {
                    ExecutionError::Invalid("LIKE pattern ends with an escape".into())
                })?;
                if matches!(escaped, b'%' | b'_' | b'\\') {
                    tokens.push(LikeToken::Byte(escaped));
                } else {
                    tokens.push(LikeToken::Byte(b'\\'));
                    tokens.push(LikeToken::Byte(escaped));
                }
            }
            literal => tokens.push(LikeToken::Byte(literal)),
        }
    }
    if let Some(bytes) = literal_bytes(&tokens) {
        return Ok(LikePattern::Exact(bytes));
    }
    if tokens.len() >= 2
        && matches!(tokens.first(), Some(LikeToken::Any))
        && matches!(tokens.last(), Some(LikeToken::Any))
    {
        if let Some(bytes) = literal_bytes(&tokens[1..tokens.len() - 1]) {
            return Ok(LikePattern::Contains(bytes));
        }
    }
    if matches!(tokens.last(), Some(LikeToken::Any)) {
        if let Some(bytes) = literal_bytes(&tokens[..tokens.len() - 1]) {
            return Ok(LikePattern::Prefix(bytes));
        }
    }
    if matches!(tokens.first(), Some(LikeToken::Any)) {
        if let Some(bytes) = literal_bytes(&tokens[1..]) {
            return Ok(LikePattern::Suffix(bytes));
        }
    }
    Ok(LikePattern::Wildcard(tokens))
}

pub(super) fn next_utf8_boundary(value: &[u8], index: usize) -> Option<usize> {
    let width = match *value.get(index)? {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let end = index.checked_add(width)?;
    std::str::from_utf8(value.get(index..end)?).ok()?;
    Some(end)
}

pub(super) fn equal_like_bytes(left: &[u8], right: &[u8], case_insensitive: bool) -> bool {
    if !case_insensitive {
        return left == right;
    }
    if left.is_ascii() && right.is_ascii() {
        return left.eq_ignore_ascii_case(right);
    }
    match (std::str::from_utf8(left), std::str::from_utf8(right)) {
        (Ok(left), Ok(right)) => left.to_lowercase() == right.to_lowercase(),
        _ => left.eq_ignore_ascii_case(right),
    }
}

fn unicode_folded_match(
    value: &[u8],
    literal: &[u8],
    compare: impl FnOnce(&str, &str) -> bool,
) -> Option<bool> {
    if value.is_ascii() && literal.is_ascii() {
        return None;
    }
    let value = std::str::from_utf8(value).ok()?;
    let literal = std::str::from_utf8(literal).ok()?;
    Some(compare(&value.to_lowercase(), &literal.to_lowercase()))
}

pub(super) fn like_matches(value: &[u8], pattern: &LikePattern, case_insensitive: bool) -> bool {
    match pattern {
        LikePattern::Exact(bytes) => equal_like_bytes(value, bytes, case_insensitive),
        LikePattern::Prefix(bytes) => {
            if case_insensitive {
                if let Some(matches) =
                    unicode_folded_match(value, bytes, |input, prefix| input.starts_with(prefix))
                {
                    return matches;
                }
            }
            value
                .get(..bytes.len())
                .is_some_and(|prefix| equal_like_bytes(prefix, bytes, case_insensitive))
        }
        LikePattern::Suffix(bytes) => {
            if case_insensitive {
                if let Some(matches) =
                    unicode_folded_match(value, bytes, |input, suffix| input.ends_with(suffix))
                {
                    return matches;
                }
            }
            value
                .get(value.len().saturating_sub(bytes.len())..)
                .is_some_and(|suffix| equal_like_bytes(suffix, bytes, case_insensitive))
        }
        LikePattern::Contains(bytes) => {
            if case_insensitive {
                if let Some(matches) =
                    unicode_folded_match(value, bytes, |input, needle| input.contains(needle))
                {
                    return matches;
                }
            }
            if bytes.is_empty() {
                true
            } else if !case_insensitive {
                // str::contains uses the standard substring search implementation
                // for valid UTF-8. Preserve byte-wise matching for raw String
                // columns that contain invalid UTF-8.
                if let (Ok(haystack), Ok(needle)) = (
                    std::str::from_utf8(value),
                    std::str::from_utf8(bytes),
                ) {
                    haystack.contains(needle)
                } else {
                    value
                        .windows(bytes.len())
                        .any(|part| part == bytes.as_slice())
                }
            } else if value.is_ascii() && bytes.is_ascii() {
                value
                    .windows(bytes.len())
                    .any(|part| part.eq_ignore_ascii_case(bytes))
            } else {
                value
                    .windows(bytes.len())
                    .any(|part| equal_like_bytes(part, bytes, true))
            }
        }
        LikePattern::Wildcard(tokens) => like_matches_wildcards(value, tokens, case_insensitive),
    }
}

pub(super) fn like_matches_wildcards(value: &[u8], pattern: &[LikeToken], case_insensitive: bool) -> bool {
    let (mut row, mut pat) = (0, 0);
    let mut last_any = None;
    let mut restart = 0;
    while row < value.len() {
        match pattern.get(pat) {
            Some(LikeToken::Byte(byte))
                if *byte == value[row]
                    || (case_insensitive && value[row].eq_ignore_ascii_case(byte)) =>
            {
                row += 1;
                pat += 1;
            }
            Some(LikeToken::One) => {
                let Some(next) = next_utf8_boundary(value, row) else {
                    return false;
                };
                row = next;
                pat += 1;
            }
            Some(LikeToken::Any) => {
                last_any = Some(pat);
                pat += 1;
                restart = row;
            }
            _ => {
                let Some(any) = last_any else {
                    return false;
                };
                let Some(next) = next_utf8_boundary(value, restart) else {
                    return false;
                };
                restart = next;
                row = restart;
                pat = any + 1;
            }
        }
    }
    while matches!(pattern.get(pat), Some(LikeToken::Any)) {
        pat += 1;
    }
    pat == pattern.len()
}

#[cfg(test)]
mod perf_tests {
    use super::*;

    #[test]
    fn case_sensitive_contains_matches_utf8_and_raw_bytes() {
        let pattern = compile_like("%needle%").unwrap();
        let mut large = "haystack".repeat(512);
        large.push_str("needle");
        assert!(like_matches(large.as_bytes(), &pattern, false));
        assert!(!like_matches(b"haystack", &pattern, false));

        let raw = b"\xffneedle\xfe";
        assert!(like_matches(raw, &pattern, false));

        let accented = compile_like("%f\u{e9}%").unwrap();
        assert!(like_matches("caf\u{e9}".as_bytes(), &accented, false));
    }

    #[test]
    fn ascii_contains_preserves_case_sensitivity() {
        let pattern = compile_like("%Needle%").unwrap();
        assert!(like_matches(b"hayNEEDLEstack", &pattern, true));
        assert!(!like_matches(b"hayNEEDLEstack", &pattern, false));
        assert!(!like_matches(b"haystack", &pattern, true));
    }

    #[test]
    fn optimized_patterns_support_expanding_unicode_case_folds() {
        for (pattern, input) in [
            ("İ%", "i\u{307}stanbul"),
            ("%İ", "city i\u{307}"),
            ("%İ%", "city i\u{307}stanbul"),
        ] {
            let compiled = compile_like(pattern).unwrap();
            assert!(like_matches(input.as_bytes(), &compiled, true));
            assert!(!like_matches(input.as_bytes(), &compiled, false));
        }
    }

    #[test]
    fn unicode_contains_keeps_case_insensitive_semantics() {
        let pattern = compile_like("%Éf%").unwrap();
        assert!(like_matches("préfixe".as_bytes(), &pattern, true));
        assert!(!like_matches("préfixe".as_bytes(), &pattern, false));
    }
}
