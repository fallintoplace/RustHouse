//! Compact-part detection must not silently treat shared streams as wide columns.
use rusthouse_storage::{NumericPartReader, PartError, TypedPartReader, is_compact_part};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn shared_streams_are_identified_before_wide_part_scan() {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rusthouse-compact-layout-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    assert!(!is_compact_part(&root).unwrap());
    fs::write(root.join("data.bin"), b"not yet a decoded compact stream").unwrap();
    assert!(!is_compact_part(&root).unwrap());
    fs::write(root.join("data.mrk3"), b"").unwrap();
    assert!(is_compact_part(&root).unwrap());
    let names = ["id".to_owned()];
    assert!(matches!(
        NumericPartReader::open(&root, &names),
        Err(PartError::InvalidScan(message)) if message.contains("compact MergeTree")
    ));
    assert!(matches!(
        TypedPartReader::open(&root, &names),
        Err(PartError::InvalidScan(message)) if message.contains("compact MergeTree")
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn incomplete_compact_marks_are_rejected() {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rusthouse-compact-missing-data-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("data.mrk3"), b"").unwrap();
    assert!(is_compact_part(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}
