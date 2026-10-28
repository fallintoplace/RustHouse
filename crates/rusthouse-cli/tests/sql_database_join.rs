use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn database() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "rusthouse-join-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn sql(query: &str, path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(path)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn joins_persisted_tables_with_duplicates_nulls_and_aliases() {
    let db = database();
    for command in [
        "CREATE TABLE users (id UInt64, team Nullable(UInt64), name String)",
        "CREATE TABLE teams (id Nullable(UInt64), label String)",
        "INSERT INTO users VALUES (1, 10, 'Ada'), (2, 10, 'Bob'), (3, NULL, 'Null')",
        "INSERT INTO teams VALUES (10, 'Ops'), (10, 'Dev'), (NULL, 'None')",
    ] {
        let result = sql(command, &db, &[]);
        assert!(result.status.success(), "{command}: {result:?}");
    }
    let join = sql(
        "SELECT u.id, t.label FROM users AS u JOIN teams t ON u.team = t.id ORDER BY u.id LIMIT 10",
        &db,
        &[],
    );
    assert!(join.status.success(), "{join:?}");
    assert_eq!(
        join.stdout,
        b"u.id\tt.label\n1\tOps\n1\tDev\n2\tOps\n2\tDev\n"
    );
    let zero = sql(
        "SELECT * FROM users u INNER JOIN teams t ON u.id = t.id LIMIT 0",
        &db,
        &[],
    );
    assert!(zero.status.success(), "{zero:?}");
    assert_eq!(zero.stdout, b"u.id\tu.team\tu.name\tt.id\tt.label\n");
    let wrong = sql(
        "SELECT * FROM users u JOIN teams t ON u.name = t.id",
        &db,
        &[],
    );
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("JOIN key types differ"));
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn inner_join_builds_more_than_one_4096_row_part() {
    let db = database();
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimensions (id UInt64)",
        "INSERT INTO facts VALUES (17), (5000)",
        "INSERT INTO dimensions SELECT number FROM numbers(4096)",
        "INSERT INTO dimensions SELECT number FROM numbers(4096, 2048)",
    ] {
        let result = sql(statement, &db, &[]);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let joined = sql(
        "SELECT f.id, d.id FROM facts f INNER JOIN dimensions d ON f.id = d.id LIMIT 10",
        &db,
        &[],
    );
    assert!(joined.status.success(), "{joined:?}");
    assert_eq!(joined.stdout, b"f.id\td.id\n17\t17\n5000\t5000\n");

    // The materializing path now handles both sides beyond 4096 rows.
    let ordered = sql(
        "SELECT f.id FROM facts f JOIN dimensions d ON f.id = d.id ORDER BY d.id LIMIT 2",
        &db,
        &[],
    );
    assert!(ordered.status.success(), "{ordered:?}");
    assert_eq!(ordered.stdout, b"f.id\n17\n5000\n");

    let outer = sql(
        "SELECT f.id, d.id FROM facts f FULL JOIN dimensions d ON f.id = d.id ORDER BY d.id LIMIT 3",
        &db,
        &[],
    );
    assert!(outer.status.success(), "{outer:?}");
    assert_eq!(outer.stdout, b"f.id\td.id\n\\N\t0\n\\N\t1\n\\N\t2\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn supports_join_explain_and_rejects_unknown_aliases() {
    let db = database();
    let explain = sql(
        "SELECT * FROM foo a JOIN bar b ON a.id = b.id",
        &db,
        &["--explain"],
    );
    assert!(explain.status.success(), "{explain:?}");
    assert!(String::from_utf8_lossy(&explain.stdout).contains("Join: Inner"));
    let unknown = sql("SELECT x.id FROM foo a JOIN bar b ON a.id = b.id", &db, &[]);
    assert!(!unknown.status.success());
}

#[test]
fn streams_large_inner_join_probe_without_buffering_left_table() {
    let db = database();
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimension (id UInt64, label String)",
        "INSERT INTO facts SELECT number FROM numbers(4096)",
        "INSERT INTO facts SELECT number FROM numbers(2048)",
        "INSERT INTO dimension VALUES (5, 'match')",
    ] {
        let result = sql(statement, &db, &[]);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let joined = sql(
        "SELECT f.id, d.label FROM facts f JOIN dimension d ON f.id = d.id LIMIT 2",
        &db,
        &[],
    );
    assert!(joined.status.success(), "{joined:?}");
    assert_eq!(joined.stdout, b"f.id\td.label\n5\tmatch\n5\tmatch\n");

    let paged = sql(
        "SELECT f.id FROM facts f INNER JOIN dimension d ON f.id = d.id LIMIT 1 OFFSET 1",
        &db,
        &[],
    );
    assert!(paged.status.success(), "{paged:?}");
    assert_eq!(paged.stdout, b"f.id\n5\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn streaming_inner_join_scans_more_than_100000_probe_rows() {
    let db = database();
    for statement in [
        "CREATE TABLE events (id UInt64)",
        "CREATE TABLE lookup (id UInt64)",
        "INSERT INTO events SELECT number FROM numbers(40000)",
        "INSERT INTO events SELECT number FROM numbers(40000, 40000)",
        "INSERT INTO events SELECT number FROM numbers(80000, 40001)",
        "INSERT INTO lookup VALUES (120000)",
    ] {
        let result = sql(statement, &db, &[]);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let joined = sql(
        "SELECT e.id FROM events e INNER JOIN lookup l ON e.id = l.id LIMIT 1",
        &db,
        &[],
    );
    assert!(joined.status.success(), "{joined:?}");
    assert_eq!(joined.stdout, b"e.id\n120000\n");
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn ordered_join_accepts_32768_rows_but_rejects_one_more() {
    let db = database();
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimensions (id UInt64)",
        "INSERT INTO dimensions VALUES (0)",
        "INSERT INTO facts SELECT number FROM numbers(0, 4096)",
        "INSERT INTO facts SELECT number FROM numbers(4096, 4096)",
        "INSERT INTO facts SELECT number FROM numbers(8192, 4096)",
        "INSERT INTO facts SELECT number FROM numbers(12288, 4096)",
        "INSERT INTO facts SELECT number FROM numbers(16384, 4096)",
        "INSERT INTO facts SELECT number FROM numbers(20480, 4096)",
        "INSERT INTO facts SELECT number FROM numbers(24576, 4096)",
        "INSERT INTO facts SELECT number FROM numbers(28672, 4096)",
    ] {
        let result = sql(statement, &db, &[]);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query =
        "SELECT f.id FROM facts f LEFT JOIN dimensions d ON f.id = d.id ORDER BY f.id LIMIT 2";
    let accepted = sql(query, &db, &[]);
    assert!(accepted.status.success(), "{accepted:?}");
    assert_eq!(accepted.stdout, b"f.id\n0\n1\n");

    let append = sql("INSERT INTO facts VALUES (32768)", &db, &[]);
    assert!(append.status.success(), "{append:?}");
    let rejected = sql(query, &db, &[]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("32768"));
    assert!(rejected.stdout.is_empty());
    fs::remove_dir_all(db).unwrap();
}
