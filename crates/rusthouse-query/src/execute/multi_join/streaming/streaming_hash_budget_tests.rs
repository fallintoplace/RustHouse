use super::*;

#[test]
fn large_string_keys_are_charged_in_addition_to_dimension_rows() {
    let key = [typed::Key::Text(vec![b'x'; 2048])];
    assert!(hash_key_bytes(&key).unwrap() > 2048);
    let mut used = MAX_CHAIN_BYTES - 100;
    assert!(charge_hash_build(&mut used, hash_key_bytes(&key).unwrap()).is_err());
    assert_eq!(used, MAX_CHAIN_BYTES - 100);
}

#[test]
fn hash_accounting_rejects_overflow_before_allocation() {
    let mut used = 0usize;
    assert!(charge_hash_build(&mut used, usize::MAX).is_err());
    assert_eq!(used, 0);
    charge_hash_build(&mut used, 128).unwrap();
    assert_eq!(used, 128);
}
