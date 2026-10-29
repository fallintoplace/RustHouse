use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn streams_clickhouse_json_each_row_from_stdin_to_persistent_table() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-insert-json-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&db);
    let exe = env!("CARGO_BIN_EXE_rusthouse");
    let created = Command::new(exe)
        .args(["sql", "CREATE TABLE events (id UInt64, title String)"])
        .args(["--database"]).arg(&db).output().unwrap();
    assert!(created.status.success(), "{created:?}");
    let mut process = Command::new(exe)
        .args(["sql", "INSERT INTO events (title, id) FORMAT JSONEachRow", "--database"])
        .arg(&db)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn().unwrap();
    let mut stdin = process.stdin.take().unwrap();
    stdin.write_all(b"{\"id\":1,\"title\":\"first\"}\n{\"title\":\"second\",\"id\":2}\n").unwrap();
    drop(stdin);
    let inserted = process.wait_with_output().unwrap();
    assert!(inserted.status.success(), "{inserted:?}");
    let selected = Command::new(exe)
        .args(["sql", "SELECT id, title FROM events ORDER BY id", "--database"])
        .arg(&db).output().unwrap();
    assert!(selected.status.success(), "{selected:?}");
    assert_eq!(selected.stdout, b"id\ttitle\n1\tfirst\n2\tsecond\n");
    fs::remove_dir_all(db).unwrap();
}
