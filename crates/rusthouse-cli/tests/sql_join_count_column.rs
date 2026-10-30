use std::fs;
use std::process::Command;

#[test]
fn streamed_count_column_skips_nulls_after_duplicate_join_matches() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-count-col-{}", std::process::id()
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64, amount Nullable(Int64))",
        "CREATE TABLE dims (id UInt64, label Nullable(String))",
        "INSERT INTO facts VALUES (1, 10), (2, NULL), (3, 3)",
        "INSERT INTO dims VALUES (1, 'A'), (1, NULL), (2, 'B')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let cases: &[(&str, &[u8])] = &[
        (
            "SELECT COUNT(f.amount) FROM facts f JOIN dims d ON f.id = d.id",
            b"count(f.amount)\n2\n",
        ),
        (
            "SELECT COUNT(d.label) FROM facts f JOIN dims d ON f.id = d.id",
            b"count(d.label)\n2\n",
        ),
        (
            "SELECT COUNT(f.id) FROM facts f JOIN dims d ON f.id = d.id",
            b"count(f.id)\n3\n",
        ),
        (
            "SELECT COUNT(d.label) FROM facts f JOIN dims d ON f.id = d.id WHERE f.amount IS NULL",
            b"count(d.label)\n1\n",
        ),
        (
            "SELECT COUNT(d.label) FROM facts f JOIN dims d ON f.id = d.id LIMIT 0",
            b"count(d.label)\n",
        ),
    ];
    for &(statement, expected) in cases {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
        assert_eq!(result.stdout, expected, "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}
