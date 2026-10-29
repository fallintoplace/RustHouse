#[test]
fn unicode_multi_percent_patterns_reuse_compiled_folded_tokens() {
    let pattern = compile_like("x%É%Y").unwrap();
    assert!(matches!(&pattern, LikePattern::Wildcard {
        folded_percent: Some(_),
        ..
    }));
    for text in ["xabcdeféghiY", "xÉY", "xémiddleY"] {
        assert!(like_matches(text.as_bytes(), &pattern, true), "{text}");
    }
    assert!(!like_matches("xabcdefémiddleY".as_bytes(), &pattern, false));
    assert!(!like_matches("xémiddleZ".as_bytes(), &pattern, true));
}

use super::*;

#[test]
fn unicode_ilike_wildcards_fold_accents_and_keep_codepoint_counts() {
    for (pattern, input, expected) in [
        ("C_FÉ", "café", true),
        ("%É_", "café!", true),
        ("%É_", "café", false),
        ("İ_", "i\u{307}x", true),
        ("_É%", "aéx", true),
        ("_É%", "aaa", false),
        ("%日本_", "xx日本語", true),
        ("_É_", "aéx", true),
        ("_É_", "aé", false),
    ] {
        let pattern = compile_like(pattern).unwrap();
        assert_eq!(like_matches(input.as_bytes(), &pattern, true), expected,
            "{input}: {expected}");
    }
    let pattern = compile_like("_É_").unwrap();
    assert!(!like_matches("aéx".as_bytes(), &pattern, false));
}

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
fn ascii_case_insensitive_single_byte_contains_scans_long_rows() {
    let pattern = compile_like("%x%").unwrap();
    let mut value = vec![b'a'; 16384];
    value.push(b'X');
    assert!(like_matches(&value, &pattern, true));
    assert!(!like_matches(&value, &pattern, false));
    assert!(!like_matches(&value[..16384], &pattern, true));
    assert!(like_matches(b"x", &pattern, true));
    assert!(!like_matches(b"", &pattern, true));
    // Non-ASCII still follows Unicode case-folding logic.
    let accent = compile_like("%é%").unwrap();
    assert!(like_matches("CAFÉ".as_bytes(), &accent, true));
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
