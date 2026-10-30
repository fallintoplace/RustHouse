use super::*;

#[test]
fn multi_percent_patterns_fold_non_ascii_literals() {
    let pattern = compile_like("%É%FIN").unwrap();
    assert!(like_matches("café au FIN".as_bytes(), &pattern, true));
    assert!(like_matches("Café au fin".as_bytes(), &pattern, true));
    assert!(!like_matches("café au FIN".as_bytes(), &pattern, false));
    assert!(!like_matches("cafe au fin".as_bytes(), &pattern, true));
}

#[test]
fn unicode_expansion_with_multiple_percent_segments() {
    let pattern = compile_like("%İ%LAND").unwrap();
    assert!(like_matches("city i\u{307} island".as_bytes(), &pattern, true));
    assert!(like_matches("city İ island".as_bytes(), &pattern, true));
    assert!(!like_matches("city izland".as_bytes(), &pattern, true));
}
