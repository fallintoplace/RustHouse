use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(statement: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn tsv_copy_streams_files_larger_than_eight_mib_and_rolls_back_late_errors() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-large-tsv-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&db).unwrap();
    let result = sql("CREATE TABLE events (id UInt64, name String)", &db);
    assert!(result.status.success(), "{result:?}");
    let source = db.join("large.tsv");
    let text = "x".repeat(160);
    {
        let mut writer = BufWriter::new(File::create(&source).unwrap());
        writeln!(writer, "id\tname").unwrap();
        for id in 0..60_000 {
            writeln!(writer, "{id}\t{text}").unwrap();
        }
        writer.flush().unwrap();
    }
    assert!(fs::metadata(&source).unwrap().len() > 8 * 1024 * 1024);
    let statement = format!(
        "COPY events FROM '{}' WITH (FORMAT TSV, HEADER TRUE)",
        source.display()
    );
    let imported = sql(&statement, &db);
    assert!(imported.status.success(), "{imported:?}");
    let count = sql("SELECT count(*) FROM events", &db);
    assert!(count.status.success(), "{count:?}");
    assert_eq!(count.stdout, b"count\n60000\n");
    let last = sql("SELECT name FROM events WHERE id = 59999", &db);
    assert!(last.status.success(), "{last:?}");
    assert_eq!(last.stdout, format!("name\n{text}\n").as_bytes());

    let pointer = db.join("events.parts/.rusthouse-active");
    let before = fs::read(&pointer).unwrap();
    {
        let mut writer = BufWriter::new(File::create(&source).unwrap());
        writeln!(writer, "id\tname").unwrap();
        for id in 0..5000 {
            writeln!(writer, "{id}\tok").unwrap();
        }
        writeln!(writer, "invalid-id\tbad").unwrap();
        writer.flush().unwrap();
    }
    let rejected = sql(&statement, &db);
    assert!(!rejected.status.success());
    assert_eq!(fs::read(&pointer).unwrap(), before);
    assert_eq!(
        sql("SELECT count(*) FROM events", &db).stdout,
        b"count\n60000\n"
    );
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn tsv_copy_groups_medium_rows_into_fewer_verified_parts() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-medium-tsv-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&db).unwrap();
    let created = sql(
        "CREATE TABLE events (a String, b String, c String, d String)",
        &db,
    );
    assert!(created.status.success(), "{created:?}");
    let input = db.join("medium.tsv");
    let value = "q".repeat(512);
    {
        let mut writer = BufWriter::new(File::create(&input).unwrap());
        for _ in 0..4000 {
            writeln!(writer, "{value}\t{value}\t{value}\t{value}").unwrap();
        }
        writer.flush().unwrap();
    }
    let copied = sql(
        &format!("COPY events FROM '{}' WITH (FORMAT TSV)", input.display()),
        &db,
    );
    assert!(copied.status.success(), "{copied:?}");
    assert_eq!(
        sql("SELECT count(*) FROM events", &db).stdout,
        b"count\n4000\n"
    );
    let parts = db.join("events.parts");
    let generation = fs::read_to_string(parts.join(".rusthouse-active")).unwrap();
    let active = parts.join(".rusthouse-versions").join(generation.trim());
    let count = fs::read_dir(active)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("all_")
        })
        .count();
    assert!(
        count <= 2,
        "expected one new data part, got {count} total parts"
    );
    fs::remove_dir_all(db).unwrap();
}
