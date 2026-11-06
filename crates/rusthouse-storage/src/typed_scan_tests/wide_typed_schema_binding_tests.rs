use super::*;
use crate::{Column, write_wide_part};
use std::fs;

#[test]
fn reversed_96_column_projection_keeps_names_types_and_value_order() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-typed-schema-binding-{}-{:?}",
        std::process::id(),
        std::thread::current().id(),
    ));
    let _ = fs::remove_dir_all(&root);
    let definitions = (0..96usize)
        .map(|index| Column {
            name: format!("metric_{index:03}"),
            data_type: "UInt64".to_owned(),
        })
        .collect::<Vec<_>>();
    let rows = (0..4usize)
        .map(|row| {
            (0..96usize)
                .map(|column| Some((row * 1000 + column).to_string()))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    write_wide_part(&root, &definitions, &rows).unwrap();
    let names = definitions
        .iter()
        .rev()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let mut reader = TypedPartReader::open(&root, &names).unwrap();
    let result = reader.next_batch().unwrap().unwrap();
    assert_eq!(result.len(), 96);
    for (index, column) in result.iter().enumerate() {
        let expected_column = 95 - index;
        let TypedColumn::Numeric(NumericColumn::UInt64(values)) = column else {
            panic!("unexpected UInt64 projection type")
        };
        assert_eq!(
            values,
            &(0..4)
                .map(|row| (row * 1000 + expected_column) as u64)
                .collect::<Vec<_>>(),
        );
    }
    assert!(reader.next_batch().unwrap().is_none());
    assert!(TypedPartReader::open(&root, &["unknown".to_owned()]).is_err());
    fs::remove_dir_all(root).unwrap();
}
