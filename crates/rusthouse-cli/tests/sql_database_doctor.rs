use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DB: AtomicUsize = AtomicUsize::new(0);

fn database() -> PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-doctor-{}-{}",
        std::process::id(),
        NEXT_DB.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(dir: &Path, statement: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(dir)
        .output()
        .unwrap()
}

fn doctor(dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("doctor")
        .arg("--database")
        .arg(dir)
        .output()
        .unwrap()
}

fn create_table(dir: &Path) {
    let created = sql(dir, "CREATE TABLE events (id UInt64)");
    assert!(created.status.success(), "{created:?}");
    let inserted = sql(dir, "INSERT INTO events VALUES (1), (2)");
    assert!(inserted.status.success(), "{inserted:?}");
}

#[test]
fn healthy_database_is_verified_without_creating_files() {
    let dir = database();
    create_table(&dir);
    let before = fs::read_dir(&dir).unwrap().count();
    let result = doctor(&dir);
    assert!(result.status.success(), "{result:?}");
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(
        output.contains("OK events: 2 rows, 2 active parts, 1 columns"),
        "{output}"
    );
    assert!(output.contains("0 error(s)"), "{output}");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), before);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bad_pointer_and_orphaned_parts_are_reported_without_repairing() {
    let dir = database();
    create_table(&dir);
    let created = sql(&dir, "CREATE TABLE lost (id UInt64)");
    assert!(created.status.success(), "{created:?}");
    fs::remove_file(dir.join("lost.schema")).unwrap();
    let pointer = dir.join("events.parts/.rusthouse-active");
    fs::write(&pointer, "../escape\n").unwrap();

    let result = doctor(&dir);
    assert!(!result.status.success(), "{result:?}");
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(
        output.contains("cannot resolve active snapshot"),
        "{output}"
    );
    assert!(output.contains("orphaned data directory"), "{output}");
    assert!(dir.join("lost.parts").exists());
    assert_eq!(fs::read_to_string(pointer).unwrap(), "../escape\n");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn corruption_and_interrupted_staging_get_distinct_diagnostics() {
    let dir = database();
    create_table(&dir);
    let interrupted = dir.join(".events.pending-orphan");
    fs::create_dir(&interrupted).unwrap();
    let stream = dir.join("events.parts/all_1_1_0/id.bin");
    let len = fs::metadata(&stream).unwrap().len() as usize;
    fs::write(&stream, vec![0u8; len]).unwrap();

    let result = doctor(&dir);
    assert!(!result.status.success(), "{result:?}");
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(
        output.contains("WARN catalog: unpublished or interrupted staging entry"),
        "{output}"
    );
    assert!(output.contains("ERROR events: invalid part"), "{output}");
    assert!(interrupted.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_database_roots_without_traversing() {
    use std::os::unix::fs::symlink;
    let dir = database();
    create_table(&dir);
    let link = dir.with_extension("link");
    symlink(&dir, &link).unwrap();
    let result = doctor(&link);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("symlinks are not supported"));
    fs::remove_file(link).unwrap();
    fs::remove_dir_all(dir).unwrap();
}
