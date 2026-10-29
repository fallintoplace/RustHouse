use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn query(db: &std::path::Path, statement: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output().unwrap()
}

fn stream(db: &std::path::Path, format: &str, content: &[u8]) -> std::process::Output {
    let mut process = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", &format!("INSERT INTO events FORMAT {format}"), "--database"])
        .arg(db)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    let mut input = process.stdin.take().unwrap();
    input.write_all(content).unwrap();
    drop(input);
    process.wait_with_output().unwrap()
}

#[test]
fn loads_clickhouse_csv_and_tsv_streams_into_persistent_tables() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-insert-stream-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    assert!(query(&db, "CREATE TABLE events (id UInt64, name String)").status.success());
    let csv = stream(&db, "CSVWithNames", b"name,id\n\"Ada, Jr\",1\n\"Bob\",2\n");
    assert!(csv.status.success(), "{csv:?}");
    let tsv = stream(&db, "TabSeparated", b"3\tCarol\n4\tDave\n");
    assert!(tsv.status.success(), "{tsv:?}");
    let output = query(&db, "SELECT id, name FROM events ORDER BY id");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"id\tname\n1\tAda, Jr\n2\tBob\n3\tCarol\n4\tDave\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn rejects_malformed_csv_without_partial_publication() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-insert-stream-atomic-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    assert!(query(&db, "CREATE TABLE events (id UInt64, name String)").status.success());
    let output = stream(&db, "CSVWithNames", b"id,name\n1,good\n2,\"unterminated\n");
    assert!(!output.status.success(), "{output:?}");
    let count = query(&db, "SELECT count(*) FROM events");
    assert_eq!(count.stdout, b"count\n0\n");
    fs::remove_dir_all(db).unwrap();
}
