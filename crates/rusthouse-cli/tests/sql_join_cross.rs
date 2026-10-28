use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(statement: &str, db: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn two_and_three_table_cross_joins_preserve_cartesian_rows() {
    let db = std::env::temp_dir().join(format!("rusthouse-cross-{}", std::process::id()));
    for statement in [
        "CREATE TABLE a (id UInt64)",
        "CREATE TABLE b (id UInt64)",
        "CREATE TABLE c (id UInt64)",
        "INSERT INTO a VALUES (1), (2)",
        "INSERT INTO b VALUES (10), (20)",
        "INSERT INTO c VALUES (100), (200)",
    ] {
        let result = sql(statement, &db, &[]);
        assert!(result.status.success(), "{statement}: {result:?}");
    }

    let two = sql(
        "SELECT a.id, b.id FROM a CROSS JOIN b ORDER BY a.id, b.id LIMIT 4",
        &db,
        &[],
    );
    assert!(two.status.success(), "{two:?}");
    assert_eq!(two.stdout, b"a.id\tb.id\n1\t10\n1\t20\n2\t10\n2\t20\n");

    let three = sql(
        "SELECT a.id, b.id, c.id FROM a CROSS JOIN b CROSS JOIN c ORDER BY a.id, b.id, c.id LIMIT 2",
        &db,
        &[],
    );
    assert!(three.status.success(), "{three:?}");
    assert_eq!(three.stdout, b"a.id\tb.id\tc.id\n1\t10\t100\n1\t10\t200\n");

    let explain = sql("SELECT a.id FROM a CROSS JOIN b", &db, &["--explain"]);
    assert!(explain.status.success(), "{explain:?}");
    assert!(String::from_utf8_lossy(&explain.stdout).contains("Cross cartesian product"));

    let invalid = sql("SELECT a.id FROM a ANY CROSS JOIN b", &db, &[]);
    assert!(!invalid.status.success());
    fs::remove_dir_all(&db).unwrap();
}
