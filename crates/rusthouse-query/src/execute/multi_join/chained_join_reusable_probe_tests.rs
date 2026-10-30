use super::*;
#[test]
fn composite_probe_keeps_vector_capacity_and_rejects_null_keys() {
    let mut probe = Vec::with_capacity(2);
    let original_capacity = probe.capacity();
    let row = [
        typed::Cell::Numeric(super::super::Number::Unsigned(7)),
        typed::Cell::Text(b"region".to_vec()),
    ];
    for _ in 0..100 {
        assert!(row_key_into(&row, &[0, 1], &mut probe));
        assert_eq!(probe, row_key(&row, &[0, 1]).unwrap());
        assert_eq!(probe.capacity(), original_capacity);
    }
    let null = [typed::Cell::Null, typed::Cell::Text(b"region".to_vec())];
    assert!(!row_key_into(&null, &[0, 1], &mut probe));
    assert!(probe.is_empty());
}
