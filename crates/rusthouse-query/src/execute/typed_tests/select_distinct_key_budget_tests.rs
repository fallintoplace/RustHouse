use super::*;

#[test]
fn text_key_bytes_contribute_to_distinct_budget() {
    let mut used = 0usize;
    let one = vec![Key::Text(vec![1u8; 1_024])];
    charge_select_distinct_key(&mut used, &one).unwrap();
    assert!(used > 1_024);
    let before = used;
    charge_select_distinct_key(&mut used, &one).unwrap();
    assert_eq!(used, before * 2);
}

#[test]
fn oversized_distinct_key_fails_without_charging() {
    let mut used = 0usize;
    let huge = vec![Key::Text(vec![b'x'; MAX_SELECT_DISTINCT_KEY_BYTES])];
    assert!(charge_select_distinct_key(&mut used, &huge).is_err());
    assert_eq!(used, 0);
}
