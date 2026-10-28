use std::fs;
use std::process::Command;

#[test]
fn exact_streaming_join_count_scans_more_than_32768_fact_rows() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-count-large-{}", std::process::id()
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimensions (id UInt64)",
        "INSERT INTO dimensions SELECT number FROM numbers(8192)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    for _ in 0..5 {
        let output = sql("INSERT INTO facts SELECT number FROM numbers(8192)");
        assert!(output.status.success(), "{output:?}");
    }
    for (statement, expected) in [
        ("SELECT COUNT(*) FROM facts f JOIN dimensions d ON f.id = d.id", b"count\n40960\n".as_slice()),
        ("SELECT COUNT(*) FROM facts f JOIN dimensions d ON f.id = d.id WHERE f.id IS NOT NULL", b"count\n40960\n"),
        ("SELECT COUNT(*) FROM facts f JOIN dimensions d ON f.id = d.id WHERE f.id IS NULL", b"count\n0\n"),
        ("SELECT COUNT(*) FROM facts f JOIN dimensions d ON f.id = d.id LIMIT 0", b"count\n"),
        ("SELECT COUNT(*) FROM facts f JOIN dimensions d ON f.id = d.id LIMIT 1 OFFSET 1", b"count\n"),
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
        assert_eq!(output.stdout, expected, "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}
