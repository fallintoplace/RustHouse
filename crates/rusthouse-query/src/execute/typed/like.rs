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

fn compiled_literal(bytes: Vec<u8>) -> LikeLiteral {
    // SQL literals do not change between rows. Do the Unicode fold once.
    let folded = std::str::from_utf8(&bytes).ok().map(str::to_lowercase);
    LikeLiteral { bytes, folded, ascii_contains_skip: None }
}

// A compiled case-insensitive ASCII contains query reuses one Horspool table
// across every scanned row instead of doing worst-case quadratic comparisons.
fn compiled_contains_literal(bytes: Vec<u8>) -> LikeLiteral {
    let mut literal = compiled_literal(bytes);
    // Retain the skip table for both ASCII ILIKE and byte-exact LIKE.
    // Even with binary haystacks, ASCII folding only reduces skip distances;
    // it can never skip a byte-exact match.
    if literal.bytes.len() >= 16 {
        let length = literal.bytes.len();
        let mut shifts = Box::new([length; 256]);
        for (index, &byte) in literal.bytes[..length - 1].iter().enumerate() {
            shifts[usize::from(byte.to_ascii_lowercase())] = length - 1 - index;
        }
        literal.ascii_contains_skip = Some(shifts);
    }
    literal
}

fn contains_ascii_folded(haystack: &[u8], needle: &[u8], shifts: &[usize; 256]) -> bool {
    debug_assert!(!needle.is_empty() && haystack.is_ascii() && needle.is_ascii());
    if haystack.len() < needle.len() {
        return false;
    }
    let last = needle.len() - 1;
    let mut start = 0;
    while start <= haystack.len() - needle.len() {
        // Horspool compares from the end. A repeated prefix with a differing
        // final byte should be rejected in O(1) comparisons per position.
        let mut remaining = needle.len();
        while remaining > 0
            && haystack[start + remaining - 1]
                .eq_ignore_ascii_case(&needle[remaining - 1])
        {
            remaining -= 1;
        }
        if remaining == 0 {
            return true;
        }
        start += shifts[usize::from(haystack[start + last].to_ascii_lowercase())];
    }
    false
}

fn contains_bytes_horspool(haystack: &[u8], needle: &[u8], shifts: &[usize; 256]) -> bool {
    debug_assert!(!needle.is_empty());
    if haystack.len() < needle.len() {
        return false;
    }
    let last = needle.len() - 1;
    let mut start = 0;
    while start <= haystack.len() - needle.len() {
        // Compare from the end, which is where the Horspool shift byte
        // lives. On a long repeated-prefix near miss (aaaa...ab against
        // aaaa...), a forward slice comparison rereads nearly the entire
        // needle at every position. Right-to-left rejects in one byte.
        let mut remaining = needle.len();
        while remaining > 0
            && haystack[start + remaining - 1] == needle[remaining - 1]
        {
            remaining -= 1;
        }
        if remaining == 0 {
            return true;
        }
        // The table is ASCII-folded so it can also serve ILIKE. A folded
        // shift is no larger than the exact-byte shift and stays safe.
        start += shifts[usize::from(haystack[start + last].to_ascii_lowercase())];
    }
    false
}

/// Compile pure percent-wildcard patterns into UTF-8 substrings once.
/// The built-in string search avoids quadratic backtracking on long text
/// columns with many overlapping literal prefixes.
fn compile_percent_segments(tokens: &[LikeToken]) -> Option<PercentSegments> {
    if tokens.iter().any(|token| matches!(token, LikeToken::One)) {
        return None;
    }
    let mut literals = Vec::new();
    let mut span = Vec::new();
    for token in tokens {
        match token {
            LikeToken::Byte(byte) => span.push(*byte),
            LikeToken::Any => {
                if !span.is_empty() {
                    literals.push(String::from_utf8(std::mem::take(&mut span)).ok()?);
                }
            }
            LikeToken::One => unreachable!(),
        }
    }
    if !span.is_empty() {
        literals.push(String::from_utf8(span).ok()?);
    }
    Some(PercentSegments {
        prefix_wildcard: matches!(tokens.first(), Some(LikeToken::Any)),
        suffix_wildcard: matches!(tokens.last(), Some(LikeToken::Any)),
        literals,
    })
}

fn match_percent_segments(value: &str, pattern: &PercentSegments) -> bool {
    let mut offset = 0usize;
    let mut first = 0usize;
    let mut end = pattern.literals.len();
    if !pattern.prefix_wildcard {
        let Some(prefix) = pattern.literals.first() else { return true; };
        if !value.starts_with(prefix.as_str()) {
            return false;
        }
        offset = prefix.len();
        first += 1;
    }
    if !pattern.suffix_wildcard {
        end = end.saturating_sub(1);
    }
    for segment in &pattern.literals[first..end] {
        let Some(found) = value[offset..].find(segment.as_str()) else { return false; };
        offset += found + segment.len();
    }
    if !pattern.suffix_wildcard {
        // The final literal must appear *after* every previously consumed
        // segment. Anchoring cannot reuse bytes from a prior segment.
        pattern.literals.last().is_some_and(|last| value[offset..].ends_with(last.as_str()))
    } else {
        true
    }
}

pub(super) fn compile_like(pattern: &str) -> Result<LikePattern, ExecutionError> {
    if !pattern.bytes().any(|byte| matches!(byte, b'%' | b'_' | b'\\')) {
        return Ok(LikePattern::Exact(compiled_literal(pattern.as_bytes().to_vec())));
    }
    // Pure prefix/suffix/contains patterns need no wildcard token vector.
    // Escapes and '_' still use the full SQL LIKE compiler below.
    if !pattern.bytes().any(|byte| matches!(byte, b'_' | b'\\')) {
        if let Some(literal) = pattern
            .strip_prefix('%')
            .and_then(|remaining| remaining.strip_suffix('%'))
        {
            if !literal.contains('%') {
                return Ok(LikePattern::Contains(compiled_contains_literal(literal.as_bytes().to_vec())));
            }
        }
        if let Some(literal) = pattern.strip_suffix('%') {
            if !literal.contains('%') {
                return Ok(LikePattern::Prefix(compiled_literal(literal.as_bytes().to_vec())));
            }
        }
        if let Some(literal) = pattern.strip_prefix('%') {
            if !literal.contains('%') {
                return Ok(LikePattern::Suffix(compiled_literal(literal.as_bytes().to_vec())));
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
        return Ok(LikePattern::Exact(compiled_literal(bytes)));
    }
    if tokens.len() >= 2
        && matches!(tokens.first(), Some(LikeToken::Any))
        && matches!(tokens.last(), Some(LikeToken::Any))
    {
        if let Some(bytes) = literal_bytes(&tokens[1..tokens.len() - 1]) {
            return Ok(LikePattern::Contains(compiled_contains_literal(bytes)));
        }
    }
    if matches!(tokens.last(), Some(LikeToken::Any)) {
        if let Some(bytes) = literal_bytes(&tokens[..tokens.len() - 1]) {
            return Ok(LikePattern::Prefix(compiled_literal(bytes)));
        }
    }
    if matches!(tokens.first(), Some(LikeToken::Any)) {
        if let Some(bytes) = literal_bytes(&tokens[1..]) {
            return Ok(LikePattern::Suffix(compiled_literal(bytes)));
        }
    }
    let folded_percent = folded_multisegment_pattern(&tokens);
    // Unicode underscore patterns can be applied to many rows. Compile the
    // folded literals once instead of rebuilding them for every match.
    let unicode_tokens = if folded_percent.is_none() {
        unicode_like_tokens(&tokens)
    } else {
        None
    };
    let percent_segments = compile_percent_segments(&tokens);
    Ok(LikePattern::Wildcard { tokens, folded_percent, unicode_tokens, percent_segments })
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

fn equal_compiled_like_bytes(value: &[u8], literal: &LikeLiteral, case_insensitive: bool) -> bool {
    if value == literal.bytes.as_slice() {
        return true;
    }
    if !case_insensitive {
        return false;
    }
    if value.eq_ignore_ascii_case(&literal.bytes) {
        return true;
    }
    if value.is_ascii() && literal.bytes.is_ascii() {
        return false;
    }
    if let (Ok(text), Some(folded)) = (std::str::from_utf8(value), literal.folded.as_deref()) {
        return text.to_lowercase() == folded;
    }
    // Keep the old ASCII fallback for invalid UTF-8 binary rows.
    false
}

fn unicode_folded_match(
    value: &[u8],
    literal: &LikeLiteral,
    compare: impl FnOnce(&str, &str) -> bool,
) -> Option<bool> {
    if value.is_ascii() && literal.bytes.is_ascii() {
        return None;
    }
    let value = std::str::from_utf8(value).ok()?;
    let folded = literal.folded.as_deref()?;
    Some(compare(&value.to_lowercase(), folded))
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
        LikePattern::Exact(literal) => equal_compiled_like_bytes(value, literal, case_insensitive),
        LikePattern::Prefix(literal) => {
            let bytes = &literal.bytes;
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
                    unicode_folded_match(value, literal, |input, prefix| input.starts_with(prefix))
                {
                    return matches;
                }
            }
            value
                .get(..bytes.len())
                .is_some_and(|prefix| equal_like_bytes(prefix, bytes, case_insensitive))
        }
        LikePattern::Suffix(literal) => {
            let bytes = &literal.bytes;
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
                    unicode_folded_match(value, literal, |input, suffix| input.ends_with(suffix))
                {
                    return matches;
                }
            }
            value
                .get(value.len().saturating_sub(bytes.len())..)
                .is_some_and(|suffix| equal_like_bytes(suffix, bytes, case_insensitive))
        }
        LikePattern::Contains(literal) => {
            let bytes = &literal.bytes;
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
                if let Some(shifts) = literal.ascii_contains_skip.as_deref() {
                    return contains_ascii_folded(value, bytes, shifts);
                }
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
                    unicode_folded_match(value, literal, |input, needle| input.contains(needle))
                {
                    return matches;
                }
            }
            if bytes.len() == 1 && !case_insensitive {
                // Avoid a substring search for the common LIKE '%x%' case.
                value.contains(&bytes[0])
            } else if !case_insensitive {
                if let Some(shifts) = literal.ascii_contains_skip.as_deref() {
                    // Long binary String values may not be UTF-8. Avoid
                    // quadratic fallback scanning for their LIKE substrings.
                    contains_bytes_horspool(value, bytes, shifts)
                } else if let (Ok(haystack), Ok(needle)) =
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
        LikePattern::Wildcard { tokens, folded_percent, unicode_tokens, percent_segments } => {
            if !case_insensitive {
                if let (Some(segments), Ok(text)) =
                    (percent_segments.as_ref(), std::str::from_utf8(value))
                {
                    return match_percent_segments(text, segments);
                }
            }
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
            if case_insensitive
                && (!value.is_ascii() || tokens.iter().any(|token| {
                    matches!(token, LikeToken::Byte(byte) if !byte.is_ascii())
                }))
            {
                if let (Ok(text), Some(cached)) =
                    (std::str::from_utf8(value), unicode_tokens.as_deref())
                {
                    return unicode_like_wildcards(text, cached);
                }
            }
            like_matches_wildcards(value, tokens, case_insensitive)
        },
    }
}

/// Preserve '_' as exactly one original Unicode codepoint, even when
/// lowercasing a character expands it into multiple codepoints.
pub(super) enum UnicodeLikeToken {
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
            LikePattern::Prefix(literal) if literal.bytes == b"hello"));
        assert!(matches!(compile_like("%world").unwrap(),
            LikePattern::Suffix(literal) if literal.bytes == b"world"));
        assert!(matches!(compile_like("%middle%").unwrap(),
            LikePattern::Contains(literal) if literal.bytes == b"middle"));
        assert!(matches!(compile_like("%").unwrap(),
            LikePattern::Prefix(literal) if literal.bytes.is_empty()));
        assert!(matches!(compile_like("h_llo%").unwrap(),
            LikePattern::Wildcard { .. }));
        assert!(like_matches(b"hello WORLD", &compile_like("%world").unwrap(), true));
        assert!(!like_matches(b"hello WORLD", &compile_like("%world").unwrap(), false));
        assert!(like_matches(b"hello WORLD", &compile_like("hello%").unwrap(), true));
    }
}

#[cfg(test)]
mod precompiled_unicode_wildcard_tests {
    use super::*;

    #[test]
    fn unicode_underscore_patterns_reuse_their_folded_literal_tokens() {
        let pattern = compile_like("MÜN_HEN").unwrap();
        assert!(matches!(&pattern, LikePattern::Wildcard {
            unicode_tokens: Some(tokens), ..
        } if !tokens.is_empty()));
        assert!(like_matches("münchen".as_bytes(), &pattern, true));
        assert!(!like_matches("münchen".as_bytes(), &pattern, false));

        let one = compile_like("STRA_E").unwrap();
        assert!(like_matches("straße".as_bytes(), &one, true));
        assert!(!like_matches("strasse".as_bytes(), &one, true));
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

#[cfg(test)]
mod empty_contains_tests {
    use super::*;

    #[test]
    fn consecutive_percent_wildcards_match_empty_and_nonempty_strings() {
        let pattern = compile_like("%%").unwrap();
        for value in [b"".as_slice(), b"one".as_slice()] {
            assert!(like_matches(value, &pattern, false));
            assert!(like_matches(value, &pattern, true));
        }
    }
}

#[cfg(test)]
mod compiled_ascii_contains_tests {
    use super::*;

    #[test]
    fn long_ascii_contains_matches_window_oracle_for_adversarial_prefixes() {
        let needles = [
            b"aaaaaaaaaaaaaaab".as_slice(),
            b"ababaabababababa",
            b"theQUICKbrownFOX",
            b"zzzzzzzzzzzzzzzz",
            b"0123456789ABCDEF",
        ];
        for needle in needles {
            let pattern = format!("%{}%", std::str::from_utf8(needle).unwrap());
            let compiled = compile_like(&pattern).unwrap();
            for length in [0usize, 1, 8, 15, 16, 31, 128, 4096] {
                for offset in [0usize, 1, 7, 13] {
                    let mut haystack = vec![b'a'; length];
                    if offset <= haystack.len() {
                        haystack.splice(offset..offset, needle.iter().copied());
                    }
                    let reference = haystack.windows(needle.len())
                        .any(|candidate| candidate.eq_ignore_ascii_case(needle));
                    assert_eq!(like_matches(&haystack, &compiled, true), reference,
                        "needle={needle:?} length={length} offset={offset}");
                }
            }
        }
    }

    #[test]
    fn compiled_skip_table_handles_uppercase_and_false_matches() {
        let needle = b"ABCDEFGHIJKLMNOP";
        let literal = compiled_contains_literal(needle.to_vec());
        let shifts = literal.ascii_contains_skip.as_deref().unwrap();
        assert!(contains_ascii_folded(b"zzabcdefghijklmnopzz", needle, shifts));
        assert!(!contains_ascii_folded(b"zzabcdefghijklmnoqzz", needle, shifts));
        assert!(!contains_ascii_folded(b"short", needle, shifts));
        assert!(compiled_contains_literal(b"short".to_vec())
            .ascii_contains_skip.is_none());
    }
}

#[cfg(test)]
mod binary_horspool_like_tests {
    use super::*;

    #[test]
    fn compiled_like_contains_matches_binary_window_oracle() {
        for literal in [
            format!("{}b", "a".repeat(31)),
            "0123456789abcdef0123456789".to_owned(),
            "AaaAAaaaAaaAAaaaAaaA".to_owned(),
        ] {
            let pattern = compile_like(&format!("%{literal}%")).unwrap();
            let needle = literal.as_bytes();
            for length in [0usize, 1, 16, 31, 32, 33, 128, 4096] {
                let mut haystack = vec![b'a'; length];
                haystack.extend_from_slice(&[0, 0xff, 0x80, b'B', 0]);
                for start in [0, 1, length / 2, length] {
                    let mut input = haystack.clone();
                    let at = start.min(input.len());
                    input.splice(at..at, needle.iter().copied());
                    let oracle = input.windows(needle.len()).any(|part| part == needle);
                    assert_eq!(like_matches(&input, &pattern, false), oracle,
                        "length={length}, at={at}");
                }
                let oracle = haystack.windows(needle.len()).any(|part| part == needle);
                assert_eq!(like_matches(&haystack, &pattern, false), oracle);
            }
        }
    }

    #[test]
    fn folded_skip_table_preserves_exact_case_sensitive_matches() {
        let needle = "AaBbCcDdEeFfGgHh";
        let pattern = compile_like(&format!("%{needle}%")).unwrap();
        assert!(like_matches(format!("xx{needle}yy").as_bytes(), &pattern, false));
        assert!(!like_matches(b"xxaabbccddeeffgghhyy", &pattern, false));
        assert!(like_matches(b"xxaabbccddeeffgghhyy", &pattern, true));
    }

    #[test]
    fn long_repeated_prefix_near_misses_remain_linear_with_binary_rows() {
        let needle = format!("{}b", "a".repeat(127));
        let pattern = compile_like(&format!("%{needle}%")).unwrap();
        let mut input = vec![b'a'; 256 * 1024];
        input.extend_from_slice(&[0xff, 0x80]);
        assert!(!like_matches(&input, &pattern, false));
        input.extend_from_slice(needle.as_bytes());
        assert!(like_matches(&input, &pattern, false));
        input.pop();
        assert!(!like_matches(&input, &pattern, false));
    }
}


#[cfg(test)]
mod multi_percent_linear_search_tests {
    use super::*;

    #[test]
    fn pure_percent_search_matches_reference_backtracker() {
        let patterns = [
            "a%b%c", "%a%b", "a%b%", "%aa%aa%ab%", "é%界%🌍",
            "%aa%ab%ba%", "a%%b%c", "%abc%abc", "a%b%c%d",
        ];
        let rows = [
            "", "abc", "axbyc", "abbc", "aabc", "aabaaba", "aabaaab",
            "éxx界yy🌍", "é🌍界", "aaaaaaab", "aaabbaa",
        ];
        for pattern in patterns {
            let compiled = compile_like(pattern).unwrap();
            let LikePattern::Wildcard { tokens, percent_segments, .. } = &compiled else {
                panic!("expected multi-wildcard compilation: {pattern}");
            };
            assert!(percent_segments.is_some());
            for row in rows {
                assert_eq!(
                    like_matches(row.as_bytes(), &compiled, false),
                    like_matches_wildcards(row.as_bytes(), tokens, false),
                    "pattern={pattern:?} row={row:?}",
                );
            }
        }
    }

    #[test]
    fn long_adversarial_prefixes_remain_exact_without_repeated_backtracking() {
        let row = format!("{}b", "a".repeat(200_000));
        let pattern = "%aaaaab%aaaaac%";
        let compiled = compile_like(pattern).unwrap();
        assert!(!like_matches(row.as_bytes(), &compiled, false));
        let matching = format!("{}b{}c", "a".repeat(30_000), "a".repeat(30_000));
        assert!(like_matches(matching.as_bytes(), &compiled, false));
    }

    #[test]
    fn invalid_utf8_rows_still_use_binary_wildcard_semantics() {
        let compiled = compile_like("a%b%c").unwrap();
        let row = b"a\xffb\xffc";
        assert!(like_matches(row, &compiled, false));
    }
}
