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
