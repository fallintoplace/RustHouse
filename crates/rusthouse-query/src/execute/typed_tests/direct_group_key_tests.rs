use super::{cell, key_at};
use rusthouse_storage::{TypedColumn, read_legacy_strings};

#[test]
fn direct_group_keys_match_materialized_cells() {
    let mut input = b"\x03foo\x00\x03foo".as_slice();
    let values = read_legacy_strings(&mut input, 3).unwrap();
    let plain = TypedColumn::Text(values);
    for row in 0..3 {
        assert!(key_at(&plain, row) == cell(&plain, row).key());
    }

    let mut input = b"\x03foo\x00\x03bar".as_slice();
    let values = read_legacy_strings(&mut input, 3).unwrap();
    let nullable = TypedColumn::NullableText {
        values,
        nulls: vec![0, 1, 0],
    };
    for row in 0..3 {
        assert!(key_at(&nullable, row) == cell(&nullable, row).key());
    }
}
