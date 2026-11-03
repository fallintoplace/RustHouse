//! One-way offline snapshot copy is groundwork, not automatic replication.
use rusthouse_storage::distributed_snapshot_copy::{
    compare_frozen_snapshot_files, copy_frozen_snapshot,
};
use rusthouse_storage::{
    Column, NumericColumn, TypedColumn, TypedPartReader, write_wide_part,
};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn verified_copy_produces_queryable_identical_replica_without_overwrite() {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rusthouse-offline-replica-{}-{unique}",
        std::process::id()
    ));
    let source = root.join("source");
    let replica = root.join("replica");
    fs::create_dir_all(&source).unwrap();

    let columns = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    let rows = vec![vec![Some("1".into())], vec![Some("2".into())]];
    write_wide_part(&source.join("all_1_1_0"), &columns, &rows).unwrap();

    let copied = copy_frozen_snapshot(&source, &replica, 4, 1024 * 1024).unwrap();
    assert_eq!(copied.parts, 1);
    assert!(copied.files > 0);

    let comparison = compare_frozen_snapshot_files(&source, &replica, 128, 1024 * 1024).unwrap();
    let same = compare_frozen_snapshot_files(&source, &source, 128, 1024 * 1024).unwrap();
    assert_eq!(comparison, same);

    let mut reader = TypedPartReader::open(&replica.join("all_1_1_0"), &["id".into()]).unwrap();
    let batch = reader.next_batch().unwrap().unwrap();
    let TypedColumn::Numeric(NumericColumn::UInt64(values)) = &batch[0] else {
        panic!("expected UInt64 replica column");
    };
    assert_eq!(values, &[1, 2]);
    assert!(copy_frozen_snapshot(&source, &replica, 4, 1024 * 1024).is_err());

    fs::remove_dir_all(root).unwrap();
}
