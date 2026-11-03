use rusthouse_storage::ensure_database_format_version;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn missing_and_v1_markers_are_compatible_but_future_versions_are_rejected() {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!(
        "rusthouse-format-compat-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    assert!(ensure_database_format_version(&root).is_ok());
    let marker = root.join(".rusthouse-format-version");
    fs::write(&marker, b"rusthouse-database-v1\n").unwrap();
    assert!(ensure_database_format_version(&root).is_ok());
    fs::write(&marker, b"rusthouse-database-v2\n").unwrap();
    assert!(ensure_database_format_version(&root).is_err());
    fs::write(&marker, b"rusthouse-database-v1\ntrailing").unwrap();
    assert!(ensure_database_format_version(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn marker_symlinks_are_not_accepted() {
    use std::os::unix::fs::symlink;
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("rusthouse-format-link-{unique}"));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("actual"), b"rusthouse-database-v1\n").unwrap();
    symlink("actual", root.join(".rusthouse-format-version")).unwrap();
    assert!(ensure_database_format_version(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}
