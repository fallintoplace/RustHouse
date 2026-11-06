use super::*;
use crate::{Column, write_wide_part};
use std::fs;

#[test]
fn wide_projection_opens_all_streams_in_order() {
    let root =
        std::env::temp_dir().join(format!("rusthouse-typed-wide-open-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let columns = (0..10)
        .map(|index| Column {
            name: format!("c{index}"),
            data_type: "UInt64".into(),
        })
        .collect::<Vec<_>>();
    let data = (0..2)
        .map(|row| {
            (0..10)
                .map(|index| Some((index + row * 100).to_string()))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    write_wide_part(&root, &columns, &data).unwrap();
    let names = columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let mut reader = TypedPartReader::open(&root, &names).unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    assert_eq!(batch.len(), 10);
    assert!(batch.iter().all(|column| column.len() == 2));
    assert!(reader.next_batch().unwrap().is_none());
    fs::remove_dir_all(root).unwrap();
}
