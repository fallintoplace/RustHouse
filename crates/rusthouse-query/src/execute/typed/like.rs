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
    // Token count cannot exceed the pattern's byte length.
    let mut tokens = Vec::with_capacity(pattern.len());
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
    let first = *value.get(index)?;
    // ASCII bytes are already valid one-byte codepoints.
    if first.is_ascii() {
        return Some(index + 1);
    }
    let width = match first {
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
    // Identical bytes have identical Unicode folds without allocations.
    if left == right {
        return true;
    }
    if !case_insensitive {
        return false;
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
            // Most matching prefixes differ only in ASCII case. Test the
            // short prefix before inspecting or folding the full UTF-8 row.
            if case_insensitive
                && bytes.is_ascii()
                && value
                    .get(..bytes.len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(bytes))
            {
                return true;
            }
            if case_insensitive && value.starts_with(bytes) {
                return true;
            }
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
            // Do not Unicode-fold or inspect an entire long string when its
            // trailing ASCII bytes already satisfy the LIKE suffix.
            if case_insensitive
                && bytes.is_ascii()
                && value
                    .get(value.len().saturating_sub(bytes.len())..)
                    .is_some_and(|suffix| suffix.eq_ignore_ascii_case(bytes))
            {
                return true;
            }
            if case_insensitive && value.ends_with(bytes) {
                return true;
            }
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
            // '%' matches every non-null String. Do not Unicode-fold every row.
            if bytes.is_empty() {
                return true;
            }
            // ILIKE '%x%' on ASCII text is a byte scan. Avoid the
            // overlapping-window comparator for each candidate byte.
            if case_insensitive && bytes.len() == 1 && bytes[0].is_ascii() && value.is_ascii() {
                return value
                    .iter()
                    .any(|byte| byte.eq_ignore_ascii_case(&bytes[0]));
            }
            if case_insensitive {
                if let Some(matches) =
                    unicode_folded_match(value, bytes, |input, needle| input.contains(needle))
                {
                    return matches;
                }
            }
            if bytes.is_empty() {
                true
            } else if bytes.len() == 1 && !case_insensitive {
                // Avoid a substring search for the common LIKE '%x%' case.
                value.contains(&bytes[0])
            } else if !case_insensitive {
                if let (Ok(haystack), Ok(needle)) =
                    (std::str::from_utf8(value), std::str::from_utf8(bytes))
                {
                    haystack.contains(needle)
                } else {
                    value.windows(bytes.len()).any(|part| part == bytes)
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

pub(super) fn like_matches_wildcards(
    value: &[u8],
    pattern: &[LikeToken],
    case_insensitive: bool,
) -> bool {
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
    fn exact_unicode_prefixes_and_suffixes_match_without_folding() {
        let prefix = compile_like("Café%").unwrap();
        let suffix = compile_like("%Café").unwrap();
        assert!(like_matches("Café au lait".as_bytes(), &prefix, true));
        assert!(like_matches("Un Café".as_bytes(), &suffix, true));
        assert!(!like_matches("un thé".as_bytes(), &suffix, true));
    }

    #[test]
    fn ascii_wildcard_steps_skip_unicode_validation() {
        assert_eq!(next_utf8_boundary(b"ascii", 0), Some(1));
        assert_eq!(next_utf8_boundary(b"ascii", 4), Some(5));
        assert_eq!(next_utf8_boundary(b"ascii", 5), None);
        assert_eq!(next_utf8_boundary("é".as_bytes(), 0), Some(2));
        assert_eq!(next_utf8_boundary(b"\xff", 0), None);
    }

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
    fn ascii_prefix_suffix_accept_case_changes_without_losing_unicode_expansions() {
        let prefix = compile_like("hello%").unwrap();
        let suffix = compile_like("%world").unwrap();
        let long_prefix = format!("HELLO{}", "x".repeat(4096));
        let long_suffix = format!("{}WORLD", "x".repeat(4096));
        assert!(like_matches(long_prefix.as_bytes(), &prefix, true));
        assert!(like_matches(long_suffix.as_bytes(), &suffix, true));
        assert!(!like_matches(long_prefix.as_bytes(), &prefix, false));
        assert!(!like_matches(long_suffix.as_bytes(), &suffix, false));
        // A Unicode lowercase mapping can expand to two codepoints.
        assert!(like_matches("İstanbul".as_bytes(), &compile_like("i%").unwrap(), true));
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
    #[test]
    fn contains_literal_handles_binary_boundaries_and_repeated_prefixes() {
        let pattern = compile_like("%ababa%").unwrap();
        assert!(like_matches(b"zzabababa", &pattern, false));
        assert!(like_matches(b"zzababab", &pattern, false));
        assert!(!like_matches(b"zzabab", &pattern, false));
        let binary = compile_like("%a\\x00b%").unwrap();
        assert!(!like_matches(b"ab", &binary, false));
        assert!(like_matches(b"prefixababa", &pattern, false));
        assert!(like_matches(
            b"anything",
            &compile_like("%%").unwrap(),
            false
        ));
    }

    #[test]
    fn empty_contains_matches_unicode_and_binary_rows() {
        let pattern = compile_like("%%").unwrap();
        assert!(like_matches("İstanbul".as_bytes(), &pattern, true));
        assert!(like_matches(b"\xff", &pattern, true));
        assert!(like_matches(b"", &pattern, true));
    }

    #[test]
    fn single_byte_contains_checks_every_position_in_binary_rows() {
        let pattern = compile_like("%x%").unwrap();
        assert!(like_matches(b"x", &pattern, false));
        assert!(like_matches(b"\0x\xff", &pattern, false));
        assert!(!like_matches(b"\0y\xff", &pattern, false));
        assert!(!like_matches(b"", &pattern, false));
        assert!(like_matches(b"X", &pattern, true));
    }

    #[test]
    fn identical_unicode_like_bytes_need_no_folding() {
        assert!(equal_like_bytes("café".as_bytes(), "café".as_bytes(), true));
        assert!(equal_like_bytes(
            "café".as_bytes(),
            "café".as_bytes(),
            false
        ));
        assert!(!equal_like_bytes(
            "café".as_bytes(),
            "cafe".as_bytes(),
            true
        ));
    }

    #[test]
    fn long_like_patterns_keep_prefix_and_escape_semantics() {
        let prefix = "x".repeat(1024);
        let pattern = compile_like(&format!("{prefix}%")).unwrap();
        assert!(like_matches(
            format!("{prefix}tail").as_bytes(),
            &pattern,
            false
        ));
        assert!(!like_matches(b"short", &pattern, false));
        let escaped = compile_like(r"left\%right").unwrap();
        assert!(like_matches(b"left%right", &escaped, false));
    }
}

#[cfg(test)]
mod single_byte_contains_tests {
    use super::*;

    #[test]
    fn case_insensitive_ascii_contains_matches_exact_character_presence() {
        let pattern = compile_like("%A%").unwrap();
        for (value, expected) in [
            ("alphabet", true), ("CAT", true), ("bbb", false),
            ("", false), ("a", true), ("zzAzz", true),
        ] {
            assert_eq!(like_matches(value.as_bytes(), &pattern, true), expected, "{value}");
        }
        assert!(!like_matches(b"zzAzz", &pattern, false));
    }
}
