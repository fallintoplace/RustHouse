use super::*;
use rusthouse_storage::read_uuid_values;

#[test]
fn build_and_probe_share_raw_uuid_keys() {
    let mut disk = [0u8; 32];
    disk[16] = 1;
    let values = read_uuid_values(&mut disk.as_slice(), 2).unwrap();
    let rows = vec![
        typed::Cell::Uuid(values[0].to_string()),
        typed::Cell::Uuid(values[1].to_string()),
    ];
    let batch = [TypedColumn::Uuid(values)];
    let build = row_key(&rows[..1], &[0]).unwrap();
    let mut reusable = Vec::new();
    assert!(batch_key_into(&batch, 0, &[0], &mut reusable));
    assert!(matches!(reusable.first(), Some(typed::Key::UuidBytes(_))));
    assert!(build == reusable);
    assert!(batch_key_into(&batch, 1, &[0], &mut reusable));
    assert!(build != reusable);
    assert!(row_key_into(&rows[1..], &[0], &mut reusable));
    assert!(reusable == row_key(&rows[1..], &[0]).unwrap());
}
