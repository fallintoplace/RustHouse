//! End-to-end coverage for cold copies of a complete local database.
//! These are offline copies: every writer has exited before each copy.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-backup-restore-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(database: &Path, statement: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

fn doctor(database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("doctor")
        .arg("--database")
        .arg(database)
        .arg("--full-hashes")
        .output()
        .unwrap()
}

fn must_succeed(output: Output) -> Output {
    assert!(output.status.success(), "{output:?}");
    output
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let target = destination.join(entry.file_name());
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            copy_directory(&source_path, &target);
        } else {
            assert!(
                kind.is_file(),
                "backup must not follow links: {source_path:?}"
            );
            fs::copy(source_path, target).unwrap();
        }
    }
}

fn populate(database: &Path) {
    must_succeed(sql(
        database,
        "CREATE TABLE events (id UInt64, name String)",
    ));
    must_succeed(sql(
        database,
        "INSERT INTO events VALUES (1, 'alpha'), (2, 'beta')",
    ));
    must_succeed(sql(database, "DELETE FROM events WHERE id = 1"));
    must_succeed(sql(database, "INSERT INTO events VALUES (3, 'gamma')"));
    must_succeed(sql(database, "OPTIMIZE TABLE events"));
}

fn active_generation(database: &Path) -> PathBuf {
    let parts = database.join("events.parts");
    let label = fs::read_to_string(parts.join(".rusthouse-active")).unwrap();
    let label = label
        .strip_suffix('\n')
        .expect("snapshot pointer ends with a newline");
    parts.join(".rusthouse-versions").join(label)
}

#[test]
fn complete_offline_copy_restores_active_rows_and_accepts_new_writes() {
    let base = root();
    fs::create_dir(&base).unwrap();
    let source = base.join("source");
    let backup = base.join("backup");
    fs::create_dir(&source).unwrap();
    populate(&source);

    let expected = must_succeed(sql(&source, "SELECT id, name FROM events ORDER BY id")).stdout;
    assert_eq!(expected, b"id\tname\n2\tbeta\n3\tgamma\n");
    must_succeed(doctor(&source));

    // A backup must include schemas, dotfiles, the active pointer, and all generations.
    copy_directory(&source, &backup);
    assert!(backup.join("events.schema").is_file());
    assert!(backup.join("events.parts/.rusthouse-active").is_file());
    assert!(active_generation(&source).is_dir());
    assert!(active_generation(&backup).is_dir());
    assert_eq!(
        fs::read(backup.join("events.parts/.rusthouse-active")).unwrap(),
        fs::read(source.join("events.parts/.rusthouse-active")).unwrap()
    );

    must_succeed(doctor(&backup));
    assert_eq!(
        must_succeed(sql(&backup, "SELECT id, name FROM events ORDER BY id")).stdout,
        expected
    );
    must_succeed(sql(&backup, "INSERT INTO events VALUES (4, 'delta')"));
    assert_eq!(
        must_succeed(sql(&backup, "SELECT count(*) FROM events")).stdout,
        b"count\n3\n"
    );
    assert_eq!(
        must_succeed(sql(&source, "SELECT id, name FROM events ORDER BY id")).stdout,
        expected
    );
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn corrupted_restored_copy_is_rejected_without_damaging_source() {
    let base = root();
    fs::create_dir(&base).unwrap();
    let source = base.join("source");
    let backup = base.join("backup");
    fs::create_dir(&source).unwrap();
    populate(&source);
    copy_directory(&source, &backup);

    let stream = fs::read_dir(active_generation(&backup))
        .unwrap()
        .map(|entry| entry.unwrap().path().join("id.bin"))
        .find(|path| path.is_file())
        .expect("active snapshot should contain a readable id stream");
    let mut data = fs::read(&stream).unwrap();
    assert!(!data.is_empty());
    let last = data.len() - 1;
    data[last] ^= 0x01;
    fs::write(&stream, data).unwrap();

    let checked = doctor(&backup);
    assert!(!checked.status.success(), "{checked:?}");
    must_succeed(doctor(&source));
    assert_eq!(
        must_succeed(sql(&source, "SELECT count(*) FROM events")).stdout,
        b"count\n2\n"
    );
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn cold_restore_preserves_renamed_table_and_latest_truncate_generation() {
    let base = root();
    fs::create_dir(&base).unwrap();
    let source = base.join("source");
    let backup = base.join("backup");
    fs::create_dir(&source).unwrap();

    must_succeed(sql(&source, "CREATE TABLE events (id UInt64, name String)"));
    must_succeed(sql(&source, "INSERT INTO events VALUES (1, 'old')"));
    must_succeed(sql(&source, "RENAME TABLE events TO archive"));
    must_succeed(sql(&source, "TRUNCATE TABLE archive"));
    must_succeed(sql(&source, "INSERT INTO archive VALUES (9, 'current')"));

    let expected = must_succeed(
        sql(&source, "SELECT id, name FROM archive ORDER BY id")
    ).stdout;
    assert_eq!(expected, b"id\tname\n9\tcurrent\n");
    assert!(!sql(&source, "SELECT id FROM events").status.success());
    must_succeed(doctor(&source));

    copy_directory(&source, &backup);
    must_succeed(doctor(&backup));
    assert_eq!(
        must_succeed(sql(&backup, "SELECT id, name FROM archive ORDER BY id")).stdout,
        expected
    );
    assert!(!sql(&backup, "SELECT id FROM events").status.success());

    must_succeed(sql(&backup, "INSERT INTO archive VALUES (10, 'restored')"));
    assert_eq!(
        must_succeed(sql(&backup, "SELECT count(*) FROM archive")).stdout,
        b"count\n2\n"
    );
    fs::remove_dir_all(base).unwrap();
}
