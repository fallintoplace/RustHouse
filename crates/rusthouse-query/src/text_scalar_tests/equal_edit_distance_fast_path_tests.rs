use super::*;

#[test]
fn equal_large_inputs_avoid_quadratic_distance_budget() {
    let text = "a".repeat(4096);
    assert_eq!(
        bounded_edit_distance(text.as_bytes(), text.as_bytes()).unwrap(),
        0
    );
    assert_eq!(bounded_utf8_edit_distance(&text, &text).unwrap(), 0);
    let utf8 = "ü".repeat(2048);
    assert_eq!(bounded_utf8_edit_distance(&utf8, &utf8).unwrap(), 0);
}

#[test]
fn unequal_large_inputs_still_obey_distance_budget() {
    let lhs = "a".repeat(2048);
    let rhs = "b".repeat(2048);
    assert!(bounded_edit_distance(lhs.as_bytes(), rhs.as_bytes()).is_err());
    assert!(bounded_utf8_edit_distance(&lhs, &rhs).is_err());
}
