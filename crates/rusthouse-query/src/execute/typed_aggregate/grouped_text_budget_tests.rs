use super::*;
#[test]
fn repeated_text_keys_are_only_charged_once_by_the_caller() {
    let mut used = 0usize;
    charge_grouped_text(&mut used, 4096).unwrap();
    assert!(used >= 8192);
    let previous = used;
    assert!(charge_grouped_text(&mut used, MAX_GROUPED_TEXT_BYTES).is_err());
    assert_eq!(used, previous);
    assert!(charge_grouped_text(&mut used, usize::MAX).is_err());
}
