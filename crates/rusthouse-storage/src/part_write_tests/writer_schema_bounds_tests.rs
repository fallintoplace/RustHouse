use super::*;

#[test]
fn rejects_zero_and_oversized_column_schemas() {
    assert!(validate_part_columns(&[]).is_err());
    let columns = (0..129)
        .map(|index| Column {
            name: format!("field_{index}"),
            data_type: "UInt64".into(),
        })
        .collect::<Vec<_>>();
    assert!(validate_part_columns(&columns).is_err());
    assert!(validate_part_columns(&columns[..128]).is_ok());
}
