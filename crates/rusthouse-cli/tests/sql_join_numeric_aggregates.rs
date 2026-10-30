use std::fs;
use std::process::Command;

#[test]
fn numeric_aggregates_stream_over_duplicate_and_nullable_join_rows() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-aggregates-{}", std::process::id()
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
        "CREATE TABLE dims (id UInt64, weight Int64)",
        "INSERT INTO facts VALUES (1, 10), (2, -5), (3, NULL)",
        "INSERT INTO dims VALUES (1, 2), (1, 3), (2, 4)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let cases: &[(&str, &[u8])] = &[
        (
            "SELECT SUM(f.amount) FROM facts f JOIN dims d ON f.id = d.id",
            b"sum(f.amount)\n15\n",
        ),
        (
            "SELECT AVG(f.amount) FROM facts f JOIN dims d ON f.id = d.id",
            b"avg(f.amount)\n5\n",
        ),
        (
            "SELECT MIN(f.amount) FROM facts f JOIN dims d ON f.id = d.id",
            b"min(f.amount)\n-5\n",
        ),
        (
            "SELECT MAX(f.amount) FROM facts f JOIN dims d ON f.id = d.id",
            b"max(f.amount)\n10\n",
        ),
        (
            "SELECT SUM(d.weight) FROM facts f JOIN dims d ON f.id = d.id",
            b"sum(d.weight)\n9\n",
        ),
        (
            "SELECT SUM(f.amount) FROM facts f ANY INNER JOIN dims d ON f.id = d.id",
            b"sum(f.amount)\n5\n",
        ),
        (
            "SELECT SUM(f.amount) FROM facts f JOIN dims d ON f.id = d.id WHERE d.weight IS NULL",
            b"sum(f.amount)\nNULL\n",
        ),
        (
            "SELECT SUM(f.amount) FROM facts f JOIN dims d ON f.id = d.id LIMIT 0",
            b"sum(f.amount)\n",
        ),
    ];
    for &(statement, expected) in cases {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
        assert_eq!(output.stdout, expected, "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}
