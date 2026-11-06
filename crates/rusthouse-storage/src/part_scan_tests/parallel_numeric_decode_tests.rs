use super::*;
use crate::{Column, write_wide_part};
use std::fs;

#[test]
fn bounded_parallel_numeric_decompress_preserves_all_column_values() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-parallel-numeric-decode-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let schema = (0..12)
        .map(|n| Column {
            name: format!("measure{n}"),
            data_type: "UInt64".into(),
        })
        .collect::<Vec<_>>();
    let rows = (0..1024)
        .map(|row| {
            (0..12)
                .map(|column| Some((row * 100 + column).to_string()))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    write_wide_part(&root, &schema, &rows).unwrap();
    let projection = schema.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
    let mut serial = NumericPartReader::open(&root, &projection).unwrap();
    let mut parallel = NumericPartReader::open(&root, &projection).unwrap();
    let expected = decode_numeric_streams(&mut serial.streams, 0, 1).unwrap();
    let actual = decode_numeric_streams(&mut parallel.streams, 0, 4).unwrap();
    assert_eq!(expected, actual);
    assert_eq!(actual.len(), 12);
    assert_eq!(actual[0].len(), 1024);
    fs::remove_dir_all(root).unwrap();
}
