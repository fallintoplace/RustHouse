use super::*;
use rusthouse_storage::read_legacy_strings;

#[test]
fn row_key_probe_reuses_storage_and_rejects_null_keys() {
    let row = [
        typed::Cell::Numeric(Number::Unsigned(7)),
        typed::Cell::Text(b"abc".to_vec()),
    ];
    let mut probe = Vec::with_capacity(2);
    assert!(row_key_into(&row, &[0, 1], &mut probe));
    let capacity = probe.capacity();
    assert!(row_key_into(&row, &[0, 1], &mut probe));
    assert_eq!(probe.capacity(), capacity);
    assert_eq!(probe, row_key(&row, &[0, 1]).unwrap());
    let invalid = [typed::Cell::Null, typed::Cell::Text(b"abc".to_vec())];
    assert!(!row_key_into(&invalid, &[0, 1], &mut probe));
    assert!(probe.is_empty());
}

#[test]
fn composite_probe_reserves_all_key_slots() {
    let values = vec![
        TypedColumn::Numeric(rusthouse_storage::NumericColumn::UInt8(vec![1])),
        TypedColumn::Numeric(rusthouse_storage::NumericColumn::UInt8(vec![2])),
        TypedColumn::Numeric(rusthouse_storage::NumericColumn::UInt8(vec![3])),
    ];
    let mut key = Vec::new();
    assert!(batch_key_into(&values, 0, &[0, 1, 2], &mut key));
    assert_eq!(key.len(), 3);
    assert!(key.capacity() >= 3);
}

#[test]
fn repeated_text_probes_reuse_the_same_allocation() {
    let mut stream = b"\x04same\x04same\x04diff".as_slice();
    let strings = read_legacy_strings(&mut stream, 3).unwrap();
    let batch = vec![TypedColumn::Text(strings)];
    let mut key = Vec::new();
    assert!(batch_key_into(&batch, 0, &[0], &mut key));
    let (address, capacity) = match &key[0] {
        typed::Key::Text(bytes) => (bytes.as_ptr(), bytes.capacity()),
        _ => panic!("expected text key"),
    };
    for index in 1..3 {
        assert!(batch_key_into(&batch, index, &[0], &mut key));
        match &key[0] {
            typed::Key::Text(bytes) => {
                assert_eq!(bytes.as_ptr(), address);
                assert_eq!(bytes.capacity(), capacity);
            }
            _ => panic!("expected text key"),
        }
    }
    assert!(matches!(&key[0], typed::Key::Text(bytes) if bytes == b"diff"));
}

#[test]
fn probe_key_moves_text_and_rejects_nullable_text() {
    let mut stream = b"\x03abc\x03def".as_slice();
    let values = read_legacy_strings(&mut stream, 2).unwrap();
    let batch = vec![TypedColumn::NullableText {
        values,
        nulls: vec![0, 1],
    }];
    let mut key = Vec::new();
    assert!(batch_key_into(&batch, 0, &[0], &mut key));
    assert!(matches!(key.as_slice(), [typed::Key::Text(value)] if value == b"abc"));
    assert!(!batch_key_into(&batch, 1, &[0], &mut key));
    assert!(key.is_empty());
}
