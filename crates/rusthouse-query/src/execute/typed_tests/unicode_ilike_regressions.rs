use super::*;

#[test]
fn matches_non_ascii_letters_in_literal_and_simple_wildcard_patterns() {
    for (pattern, text) in [
        ("CAFÉ", "café"),
        ("CAFÉ%", "café au lait"),
        ("%CAFÉ", "un café"),
        ("%CAFÉ%", "un café noir"),
        ("İ", "i\u{307}"),
    ] {
        let compiled = compile_like(pattern).unwrap();
        assert!(like_matches(text.as_bytes(), &compiled, true), "{pattern}");
        assert!(
            !like_matches(text.as_bytes(), &compiled, false),
            "{pattern}"
        );
    }
}

#[test]
fn keeps_ascii_behavior_and_does_not_match_invalid_utf8() {
    let pattern = compile_like("ABC%").unwrap();
    assert!(like_matches(b"abcdef", &pattern, true));
    assert!(!like_matches(&[0xff], &compile_like("É").unwrap(), true));
}
