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
    if !pattern.bytes().any(|byte| matches!(byte, b'%' | b'_' | b'\\')) {
        return Ok(LikePattern::Exact(pattern.as_bytes().to_vec()));
    }
    // Pure prefix/suffix/contains patterns need no wildcard token vector.
    // Escapes and '_' still use the full SQL LIKE compiler below.
    if !pattern.bytes().any(|byte| matches!(byte, b'_' | b'\\')) {
        if let Some(literal) = pattern
            .strip_prefix('%')
            .and_then(|remaining| remaining.strip_suffix('%'))
        {
            if !literal.contains('%') {
                return Ok(LikePattern::Contains(literal.as_bytes().to_vec()));
            }
        }
        if let Some(literal) = pattern.strip_suffix('%') {
            if !literal.contains('%') {
                return Ok(LikePattern::Prefix(literal.as_bytes().to_vec()));
            }
        }
        if let Some(literal) = pattern.strip_prefix('%') {
            if !literal.contains('%') {
                return Ok(LikePattern::Suffix(literal.as_bytes().to_vec()));
            }
        }
    }
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
    let folded_percent = folded_multisegment_pattern(&tokens);
    Ok(LikePattern::Wildcard { tokens, folded_percent })
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
    // Non-ASCII bytes can stay unchanged while surrounding ASCII letters
    // differ only in case. Try a borrowed byte comparison before allocating
    // two Unicode-lowercased Strings.
    if left.eq_ignore_ascii_case(right) {
        return true;
    }
    if left.is_ascii() && right.is_ascii() {
        return false;
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

/// Rebuild literal runs after Unicode case folding while keeping LIKE wildcards.
/// The pure-% variant is safe because it does not change underscore's
/// one-original-character semantics when lowercasing expands a character.
fn folded_multisegment_pattern(pattern: &[LikeToken]) -> Option<Vec<LikeToken>> {
    let mut folded = Vec::with_capacity(pattern.len());
    let mut literal = Vec::new();
    let flush = |literal: &mut Vec<u8>, folded: &mut Vec<LikeToken>| -> Option<()> {
        let text = std::str::from_utf8(literal).ok()?;
        folded.extend(text.to_lowercase().bytes().map(LikeToken::Byte));
        literal.clear();
        Some(())
    };
    for token in pattern {
        match token {
            LikeToken::Byte(byte) => literal.push(*byte),
            LikeToken::Any => {
                flush(&mut literal, &mut folded)?;
                folded.push(LikeToken::Any);
            }
            LikeToken::One => return None,
        }
    }
    flush(&mut literal, &mut folded)?;
    Some(folded)
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
            if value.len() < bytes.len()
                && (!case_insensitive || (value.is_ascii() && bytes.is_ascii()))
            {
                return false;
            }
            // Check the one-byte case before the generic ASCII window scan.
            // The old ordering made this fast path unreachable for ASCII rows.
            if case_insensitive && bytes.len() == 1 && bytes[0].is_ascii() && value.is_ascii() {
                return value
                    .iter()
                    .any(|byte| byte.eq_ignore_ascii_case(&bytes[0]));
            }
            // ASCII-only ILIKE can search borrowed bytes directly, without
            // decoding or allocating a Unicode-folded copy of every row.
            if case_insensitive && value.is_ascii() && bytes.is_ascii() {
                return value
                    .windows(bytes.len())
                    .any(|candidate| candidate.eq_ignore_ascii_case(bytes));
            }
            if case_insensitive
                && bytes.len() == 1
                && bytes[0].is_ascii()
                && value.iter().any(|byte| byte.eq_ignore_ascii_case(&bytes[0]))
            {
                return true;
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
        LikePattern::Wildcard { tokens, folded_percent } => {
            // Multi-segment ILIKE patterns must fold Unicode as a whole.
            // ASCII and '_' patterns keep the allocation-free scan.
            if case_insensitive
                && (!value.is_ascii() || tokens.iter().any(|token| {
                    matches!(token, LikeToken::Byte(byte) if !byte.is_ascii())
                }))
                && !tokens.iter().any(|token| matches!(token, LikeToken::One))
            {
                if let (Ok(text), Some(pattern)) = (
                    std::str::from_utf8(value),
                    folded_percent.as_ref(),
                ) {
                    return like_matches_wildcards(
                        text.to_lowercase().as_bytes(),
                        &pattern,
                        false,
                    );
                }
            }
            like_matches_wildcards(value, tokens, case_insensitive)
        },
    }
}

/// Preserve '_' as exactly one original Unicode codepoint, even when
/// lowercasing a character expands it into multiple codepoints.
enum UnicodeLikeToken {
    Literal(String),
    Any,
    One,
}

fn unicode_like_tokens(pattern: &[LikeToken]) -> Option<Vec<UnicodeLikeToken>> {
    let mut result = Vec::new();
    let mut literal = Vec::new();
    let flush = |result: &mut Vec<UnicodeLikeToken>, literal: &mut Vec<u8>| -> Option<()> {
        if !literal.is_empty() {
            let value = std::str::from_utf8(literal).ok()?;
            result.push(UnicodeLikeToken::Literal(value.to_lowercase()));
            literal.clear();
        }
        Some(())
    };
    for token in pattern {
        match token {
            LikeToken::Byte(byte) => literal.push(*byte),
            LikeToken::Any => {
                flush(&mut result, &mut literal)?;
                result.push(UnicodeLikeToken::Any);
            }
            LikeToken::One => {
                flush(&mut result, &mut literal)?;
                result.push(UnicodeLikeToken::One);
            }
        }
    }
    flush(&mut result, &mut literal)?;
    Some(result)
}

fn folded_literal_prefix(value: &str, literal: &str) -> Option<usize> {
    let mut matched = 0usize;
    let mut consumed = 0usize;
    for character in value.chars() {
        for lowercase in character.to_lowercase() {
            let mut encoded = [0u8; 4];
            let unit = lowercase.encode_utf8(&mut encoded).as_bytes();
            if literal.as_bytes().get(matched..matched + unit.len())? != unit {
                return None;
            }
            matched += unit.len();
        }
        consumed += character.len_utf8();
        if matched == literal.len() {
            return Some(consumed);
        }
    }
    None
}

fn unicode_like_wildcards(value: &str, pattern: &[UnicodeLikeToken]) -> bool {
    let (mut row, mut pat) = (0usize, 0usize);
    let mut last_any = None;
    let mut restart = 0usize;
    while row < value.len() {
        match pattern.get(pat) {
            Some(UnicodeLikeToken::Literal(literal)) => {
                if let Some(size) = folded_literal_prefix(&value[row..], literal) {
                    row += size;
                    pat += 1;
                    continue;
                }
            }
            Some(UnicodeLikeToken::One) => {
                row += value[row..].chars().next().expect("valid character boundary").len_utf8();
                pat += 1;
                continue;
            }
            Some(UnicodeLikeToken::Any) => {
                last_any = Some(pat);
                pat += 1;
                restart = row;
                continue;
            }
            None => {}
        }
        let Some(any) = last_any else { return false; };
        let Some(character) = value[restart..].chars().next() else { return false; };
        restart += character.len_utf8();
        row = restart;
        pat = any + 1;
    }
    while matches!(pattern.get(pat), Some(UnicodeLikeToken::Any)) { pat += 1; }
    pat == pattern.len()
}

pub(super) fn like_matches_wildcards(
    value: &[u8],
    pattern: &[LikeToken],
    case_insensitive: bool,
) -> bool {
    if case_insensitive && (!value.is_ascii()
        || pattern.iter().any(|token| matches!(token, LikeToken::Byte(byte) if !byte.is_ascii())))
    {
        if let Ok(text) = std::str::from_utf8(value) {
            if let Some(tokens) = unicode_like_tokens(pattern) {
                return unicode_like_wildcards(text, &tokens);
            }
        }
    }
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
mod mixed_case_unicode_equality_tests {
    use super::*;

    #[test]
    fn ascii_case_changes_near_accents_do_not_require_unicode_folding() {
        assert!(equal_like_bytes("Café".as_bytes(), "café".as_bytes(), true));
        assert!(equal_like_bytes("CAFÉ".as_bytes(), "cafÉ".as_bytes(), true));
        assert!(!equal_like_bytes("Café".as_bytes(), "café".as_bytes(), false));
        assert!(equal_like_bytes("MÜNCHEN".as_bytes(), "münchen".as_bytes(), true));
        assert!(!equal_like_bytes("Café".as_bytes(), "cafè".as_bytes(), true));
    }
}

#[cfg(test)]
mod ascii_single_byte_contains_tests {
    use super::*;

    #[test]
    fn case_insensitive_single_byte_contains_handles_ascii_and_unicode() {
        let pattern = compile_like("%x%").unwrap();
        assert!(like_matches(b"some XPRESS word", &pattern, true));
        assert!(like_matches(b"lower x", &pattern, true));
        assert!(!like_matches(b"missing", &pattern, true));
        assert!(!like_matches(b"missing", &pattern, false));
        assert!(like_matches("préfiX".as_bytes(), &pattern, true));
        assert!(!like_matches("café".as_bytes(), &pattern, true));
    }
}

#[cfg(test)]
mod simple_like_compile_tests {
    use super::*;

    #[test]
    fn common_literal_patterns_skip_wildcard_tokenization() {
        assert!(matches!(compile_like("hello%").unwrap(),
            LikePattern::Prefix(bytes) if bytes == b"hello"));
        assert!(matches!(compile_like("%world").unwrap(),
            LikePattern::Suffix(bytes) if bytes == b"world"));
        assert!(matches!(compile_like("%middle%").unwrap(),
            LikePattern::Contains(bytes) if bytes == b"middle"));
        assert!(matches!(compile_like("%").unwrap(),
            LikePattern::Prefix(bytes) if bytes.is_empty()));
        assert!(matches!(compile_like("h_llo%").unwrap(),
            LikePattern::Wildcard { .. }));
        assert!(like_matches(b"hello WORLD", &compile_like("%world").unwrap(), true));
        assert!(!like_matches(b"hello WORLD", &compile_like("%world").unwrap(), false));
        assert!(like_matches(b"hello WORLD", &compile_like("hello%").unwrap(), true));
    }
}

#[cfg(test)]
mod ascii_contains_unicode_tests {
    use super::*;

    #[test]
    fn mixed_unicode_ascii_substrings_match_without_changing_folds() {
        let ascii = compile_like("%x%").unwrap();
        assert!(like_matches("préfiX café".as_bytes(), &ascii, true));
        assert!(!like_matches("préfiX café".as_bytes(), &ascii, false));
        assert!(!like_matches("café".as_bytes(), &ascii, true));
        let expanding = compile_like("%i%").unwrap();
        assert!(like_matches("İstanbul".as_bytes(), &expanding, true));
    }
}

#[cfg(test)]
#[path = "like/perf_tests.rs"]
mod perf_tests;

#[cfg(test)]
#[path = "like/single_byte_contains_tests.rs"]
mod single_byte_contains_tests;

#[cfg(test)]
#[path = "like/unicode_multisegment_ilike_tests.rs"]
mod unicode_multisegment_ilike_tests;

#[cfg(test)]
mod ascii_contains_fast_path_tests {
    use super::*;

    #[test]
    fn borrowed_ascii_substring_comparison_preserves_ilike() {
        let pattern = compile_like("%AbC%").unwrap();
        assert!(like_matches(b"zabcZ", &pattern, true));
        assert!(like_matches(b"ABC", &pattern, true));
        assert!(!like_matches(b"AB", &pattern, true));
        assert!(!like_matches(b"zzzzzz", &pattern, true));
        assert!(!like_matches(b"zabcZ", &pattern, false));
    }

    #[test]
    fn non_ascii_values_still_use_unicode_case_folding() {
        let pattern = compile_like("%CAFÉ%").unwrap();
        assert!(like_matches("un café au lait".as_bytes(), &pattern, true));
    }
}
