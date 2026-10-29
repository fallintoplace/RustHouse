use rusthouse_storage::{Column, NumericColumn, TypedColumn, TypedPartReader, write_wide_part, verify_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn root() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-columnwise-part-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).unwrap();
    path
}

#[test]
fn columnwise_writer_preserves_wide_and_nullable_granules() {
    let root = root();
    let part = root.join("all_1_1_0");
    let mut columns = vec![
        Column { name: "id".into(), data_type: "UInt64".into() },
        Column { name: "description".into(), data_type: "Nullable(String)".into() },
    ];
    for index in 0..48 {
        columns.push(Column { name: format!("measure{index}"), data_type: "Int64".into() });
    }
    let rows = (0..2049).map(|index| {
        let mut row = vec![
            Some(index.to_string()),
            (index % 5 != 0).then(|| format!("long-value-{index}-{}", "x".repeat(100))),
        ];
        for metric in 0..48 {
            row.push(Some((index as i64 - metric).to_string()));
        }
        row
    }).collect::<Vec<_>>();
    write_wide_part(&part, &columns, &rows).unwrap();
    verify_wide_part(&part).unwrap();
    let mut scan = TypedPartReader::open(
        &part, &["id".to_owned(), "description".to_owned(), "measure47".to_owned()]
    ).unwrap();
    let mut total = 0;
    while let Some(batch) = scan.next_batch().unwrap() {
        let TypedColumn::Numeric(NumericColumn::UInt64(ids)) = &batch[0] else {
            panic!("expected UInt64");
        };
        for (offset, value) in ids.iter().enumerate() {
            assert_eq!(*value, (total + offset) as u64);
        }
        total += ids.len();
    }
    assert_eq!(total, 2049);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_cell_encoding_does_not_publish_partial_part_directory() {
    let root = root();
    let part = root.join("all_2_2_0");
    let columns = [
        Column { name: "good".into(), data_type: "String".into() },
        Column { name: "bad".into(), data_type: "UInt8".into() },
    ];
    let rows = [vec![Some("hello".into()), Some("not-an-integer".into())]];
    assert!(write_wide_part(&part, &columns, &rows).is_err());
    assert!(!part.exists(), "incomplete part must be removed on failure");
    fs::remove_dir_all(root).unwrap();
}
