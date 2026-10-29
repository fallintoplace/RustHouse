use super::*;

#[test]
fn rejects_bounded_row_overcommit_without_modifying_accounting() {
    let old = 20;
    assert_eq!(updated_sort_bytes(old, 10, 12).unwrap(), 22);
    assert!(updated_sort_bytes(MAX_SORT_BUFFER_BYTES, 0, 1).is_err());
    assert!(updated_sort_bytes(0, 1, 0).is_err());
    let mut sorted = SortedRows::new(Some(2), false);
    sorted.push(Number::Unsigned(1), vec![Number::Unsigned(1); 4]).unwrap();
    sorted.push(Number::Unsigned(2), vec![Number::Unsigned(2); 4]).unwrap();
    assert!(sorted.retained_bytes > 0);
    let previous = sorted.retained_bytes;
    sorted.push(Number::Unsigned(99), vec![Number::Unsigned(9); 4]).unwrap();
    assert_eq!(sorted.retained_bytes, previous);
}

#[test]
fn numeric_row_estimate_includes_extra_sort_keys() {
    let row = SortedRow {
        key: Number::Unsigned(1),
        extra_keys: vec![Number::Unsigned(2)],
        values: vec![Number::Unsigned(3); 4],
        sequence: 0,
        descending: false,
        extra_descending: Arc::from(Vec::<bool>::new()),
    };
    assert!(retained_row_size(&row).unwrap() >= std::mem::size_of::<SortedRow>()
        + 5 * std::mem::size_of::<Number>());
}
