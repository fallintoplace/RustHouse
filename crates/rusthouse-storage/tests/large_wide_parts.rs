use rusthouse_storage::{
    Column, NumericColumn, NumericPartReader, inspect_part, read_wide_marks, verify_wide_part,
    write_wide_part,
};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_TEST: AtomicUsize = AtomicUsize::new(0);

#[test]
fn writes_and_reads_a_262144_row_part() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-large-wide-part-{}-{}",
        std::process::id(),
        NEXT_TEST.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let columns = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    let rows = (0..262_144u64)
        .map(|id| vec![Some(id.to_string())])
        .collect::<Vec<_>>();
    let part = root.join("all_1_1_0");
    write_wide_part(&part, &columns, &rows).unwrap();
    verify_wide_part(&part).unwrap();
    assert_eq!(inspect_part(&part).unwrap().rows, 262_144);
    let marks = read_wide_marks(part.join("id.mrk2")).unwrap();
    assert_eq!(marks.marks.len(), 256);
    assert!(marks.marks.iter().all(|mark| mark.rows == Some(1024)));

    let mut reader = NumericPartReader::open(&part, &["id".into()]).unwrap();
    let mut seen = 0u64;
    while let Some(batch) = reader.next_batch().unwrap() {
        assert_eq!(batch.len(), 1);
        match &batch[0] {
            NumericColumn::UInt64(values) => {
                for &id in values {
                    assert_eq!(id, seen);
                    seen += 1;
                }
            }
            _ => panic!("expected UInt64 granule"),
        }
    }
    assert_eq!(seen, 262_144);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_more_than_262144_rows_before_creating_part() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-wide-part-limit-{}-{}",
        std::process::id(),
        NEXT_TEST.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let columns = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    let rows = vec![vec![Some("1".to_string())]; 262_145];
    let part = root.join("all_1_1_0");
    assert!(write_wide_part(&part, &columns, &rows).is_err());
    assert!(!part.exists());
    fs::remove_dir_all(root).unwrap();
}
