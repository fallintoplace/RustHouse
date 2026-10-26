use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn catalog() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-ddl-show-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(text: &str, dir: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", text, "--catalog"])
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn show_tables_lists_regular_schemas_in_sorted_order() {
    let dir = catalog();
    for name in ["zeta", "alpha", "beta"] {
        let created = sql(&format!("CREATE TABLE {name} (id UInt64)"), &dir);
        assert!(created.status.success(), "{created:?}");
    }
    fs::write(dir.join("ignored.txt"), "ignored").unwrap();
    fs::write(dir.join("bad-name.schema"), "ignored").unwrap();
    fs::create_dir(dir.join("folder.schema")).unwrap();

    let listed = sql("SHOW TABLES;", &dir);
    assert!(listed.status.success(), "{listed:?}");
    assert_eq!(String::from_utf8(listed.stdout).unwrap(), "name\nalpha\nbeta\nzeta\n");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn show_tables_accepts_missing_catalog_and_rejects_trailing_sql() {
    let dir = catalog();
    let empty = sql("SHOW TABLES", &dir);
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(String::from_utf8(empty.stdout).unwrap(), "name\n");
    assert!(!dir.exists());

    let invalid = sql("SHOW TABLES; DROP TABLE x", &dir);
    assert!(!invalid.status.success());
    assert!(!dir.exists());
}

#[cfg(unix)]
#[test]
fn show_tables_does_not_follow_catalog_symlinks() {
    let dir = catalog();
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("real.txt"), "untouched").unwrap();
    std::os::unix::fs::symlink(dir.join("real.txt"), dir.join("linked.schema")).unwrap();
    let listed = sql("SHOW TABLES", &dir);
    assert!(listed.status.success(), "{listed:?}");
    assert_eq!(String::from_utf8(listed.stdout).unwrap(), "name\n");
    fs::remove_dir_all(dir).unwrap();
}
