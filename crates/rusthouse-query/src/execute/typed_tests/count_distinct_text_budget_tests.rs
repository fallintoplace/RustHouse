use super::*;

#[test]
fn duplicate_strings_do_not_charge_memory_twice() {
    let mut seen = HashSet::new();
    let mut retained = 0usize;
    remember_distinct_text(&mut seen, &mut retained, b"same").unwrap();
    let charged = retained;
    remember_distinct_text(&mut seen, &mut retained, b"same").unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(retained, charged);
}

#[test]
fn rejects_oversized_new_text_before_allocating_hash_entry() {
    let mut seen = HashSet::new();
    let mut retained = 0usize;
    let value = vec![b'x'; MAX_TEXT_DISTINCT_BYTES];
    assert!(remember_distinct_text(&mut seen, &mut retained, &value).is_err());
    assert_eq!(retained, 0);
    assert!(seen.is_empty());
}
