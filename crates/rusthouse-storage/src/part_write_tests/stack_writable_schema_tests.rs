use super::*;

#[test]
fn fully_populated_columns_reject_duplicates_without_hashing() {
    let names = (0..128usize)
        .map(|index| Column {
            name: format!("column_{index:03}"),
            data_type: "UInt64".into(),
        })
        .collect::<Vec<_>>();
    assert!(validate_part_columns(&names).is_ok());
    let mut reversed = names.clone();
    reversed.reverse();
    assert!(validate_part_columns(&reversed).is_ok());
    for duplicate_at in [0usize, 64, 127] {
        let mut invalid = names.clone();
        invalid[duplicate_at].name = names[(duplicate_at + 1) % 128].name.clone();
        assert!(validate_part_columns(&invalid).is_err());
    }
    let mut wrong_type = names;
    wrong_type[93].data_type = "UnsupportedType".into();
    assert!(validate_part_columns(&wrong_type).is_err());
}
