//! Large physical scans remain exact across packed MergeTree mark blocks.
use rusthouse_storage::{
    Column, NumericColumn, NumericPartReader, inspect_part, read_wide_marks,
    verify_wide_part, write_wide_part,
};
use std::fs;

#[test]
fn packed_blocks_span_marks_and_preserve_full_physical_scan() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-packed-marks-{}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let part = root.join("all_1_1_0");
    let columns = [Column { name: "id".into(), data_type: "UInt64".into() }];
    let rows = (0..24_576u64)
        .map(|id| vec![Some(id.to_string())])
        .collect::<Vec<_>>();
    write_wide_part(&part, &columns, &rows).unwrap();
    assert_eq!(inspect_part(&part).unwrap().rows, 24_576);
    verify_wide_part(&part).unwrap();
    let marks = read_wide_marks(part.join("id.mrk2")).unwrap();
    assert_eq!(marks.marks.len(), 24);
    assert_eq!(marks.marks[0].compressed_offset, marks.marks[1].compressed_offset);
    assert!(marks.marks[1].decompressed_offset > 0);
    assert!(marks.marks[8].compressed_offset > marks.marks[7].compressed_offset);
    let mut reader = NumericPartReader::open(&part, &["id".into()]).unwrap();
    let mut seen = 0_u64;
    while let Some(batch) = reader.next_batch().unwrap() {
        match &batch[0] {
            NumericColumn::UInt64(values) => {
                for &value in values {
                    assert_eq!(value, seen);
                    seen += 1;
                }
            }
            other => panic!("unexpected physical numeric column: {other:?}"),
        }
    }
    assert_eq!(seen, 24_576);
    fs::remove_dir_all(root).unwrap();
}
