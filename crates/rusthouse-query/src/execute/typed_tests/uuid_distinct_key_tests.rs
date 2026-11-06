use super::{Key, key_at};
use rusthouse_storage::{TypedColumn, read_uuid_values};

#[test]
fn distinct_probe_uses_canonical_uuid_bytes_without_formatting() {
    let mut disk = [0u8; 48];
    disk[16] = 1;
    let values = read_uuid_values(&mut disk.as_slice(), 3).unwrap();
    let column = TypedColumn::Uuid(values);
    let first = key_at(&column, 0);
    let second = key_at(&column, 1);
    assert!(matches!(&first, Key::UuidBytes(_)));
    assert!(first != second);
    assert!(first == key_at(&column, 2));
}
