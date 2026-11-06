use super::*;
use crate::{Column, write_wide_part};
use std::fs;

#[test]
fn narrow_and_wide_projections_resolve_columns_in_requested_order() {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-narrow-projection-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&part);
    let schema = (0..32usize)
        .map(|index| Column {
            name: format!("measure_{index:02}"),
            data_type: "UInt64".into(),
        })
        .collect::<Vec<_>>();
    let rows = (0..3usize)
        .map(|row| {
            (0..32)
                .map(|column| Some((row * 100 + column).to_string()))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    write_wide_part(&part, &schema, &rows).unwrap();
    for projection in [vec![31usize], vec![7, 3, 31, 0], vec![7, 3, 31, 0, 8]] {
        let names = projection
            .iter()
            .map(|&index| schema[index].name.clone())
            .collect::<Vec<_>>();
        let mut reader = NumericPartReader::open(&part, &names).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        for (position, &index) in projection.iter().enumerate() {
            assert_eq!(
                batch[position],
                NumericColumn::UInt64((0..3).map(|row| row * 100 + index as u64).collect())
            );
        }
        assert!(reader.next_batch().unwrap().is_none());
    }
    assert!(NumericPartReader::open(&part, &["not_a_column".into()]).is_err());
    fs::remove_dir_all(part).unwrap();
}
