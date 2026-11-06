use super::*;

#[test]
fn charges_both_owned_copies_before_insert() {
    let mut charged = 0;
    charge_grouped_text(&mut charged, 1024).unwrap();
    assert!(charged >= 2048);
    let previous = charged;
    assert!(charge_grouped_text(&mut charged, MAX_GROUPED_TEXT_BYTES).is_err());
    assert_eq!(charged, previous);
    assert!(charge_grouped_text(&mut charged, usize::MAX).is_err());
}
