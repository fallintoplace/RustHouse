use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn sql(statement: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn imports_clickhouse_json_each_row_with_column_reordering_and_nullable_fields() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-json-each-row-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let db = dir.join("db");
    let data = dir.join("rows.jsonl");
    assert!(sql("CREATE TABLE events (id UInt64, name String, note Nullable(String))", &db).status.success());
    fs::write(
        &data,
        concat!(
            "{\"name\":\"Ada\",\"id\":1,\"note\":null}\n",
            "{\"note\":\"hello\",\"id\":2,\"name\":\"Bob\"}\n",
            "{\"id\":3,\"name\":\"café\\uD83D\\uDE80\"}\n",
        ),
    ).unwrap();
    let imported = sql(
        &format!("COPY events FROM '{}' WITH (FORMAT JSONEachRow)", data.display()), &db,
    );
    assert!(imported.status.success(), "{imported:?}");
    let selected = sql("SELECT id, name, note FROM events ORDER BY id", &db);
    assert!(selected.status.success(), "{selected:?}");
    assert_eq!(selected.stdout, "id\tname\tnote\n1\tAda\t\\N\n2\tBob\thello\n3\tcafé🚀\t\\N\n".as_bytes());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_second_row_does_not_publish_a_partial_import() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-json-each-row-atomic-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let db = dir.join("db");
    let data = dir.join("rows.jsonl");
    assert!(sql("CREATE TABLE events (id UInt64, name String)", &db).status.success());
    fs::write(&data, "{\"id\":1,\"name\":\"valid\"}\n{\"id\":2,\"id\":3}\n").unwrap();
    let invalid = sql(&format!(
        "COPY events FROM '{}' WITH (FORMAT JSONEachRow)", data.display()
    ), &db);
    assert!(!invalid.status.success(), "{invalid:?}");
    let count = sql("SELECT count(*) FROM events", &db);
    assert!(count.status.success(), "{count:?}");
    assert_eq!(count.stdout, b"count\n0\n");
    fs::remove_dir_all(dir).unwrap();
}
