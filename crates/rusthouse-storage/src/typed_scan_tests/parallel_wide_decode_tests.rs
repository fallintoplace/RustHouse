use super::*;
use crate::{Column, write_wide_part};
use std::fs;

#[test]
fn parallel_columns_match_serial_granule_decode_and_projection_order() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-typed-wide-decode-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let columns = (0..12)
        .map(|i| Column {
            name: format!("metric{i}"),
            data_type: "UInt64".into(),
        })
        .collect::<Vec<_>>();
    let rows = (0..1024)
        .map(|i| {
            (0..12)
                .map(|column| Some((i * 1000 + column).to_string()))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    write_wide_part(&root, &columns, &rows).unwrap();
    let projection = columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let mut serial = TypedPartReader::open(&root, &projection).unwrap();
    let mut parallel = TypedPartReader::open(&root, &projection).unwrap();
    let left = decode_typed_columns(&mut serial.columns, 0, 1024, 1).unwrap();
    let right = decode_typed_columns(&mut parallel.columns, 0, 1024, 4).unwrap();
    assert_eq!(left.len(), 12);
    for (column, (a, b)) in left.into_iter().zip(right).enumerate() {
        match (a, b) {
            (
                TypedColumn::Numeric(NumericColumn::UInt64(a)),
                TypedColumn::Numeric(NumericColumn::UInt64(b)),
            ) => {
                assert_eq!(a, b);
                assert_eq!(a[0], column as u64);
                assert_eq!(a[1023], 1_023_000 + column as u64);
            }
            _ => panic!("wrong wide projection column type"),
        }
    }
    fs::remove_dir_all(root).unwrap();
}
