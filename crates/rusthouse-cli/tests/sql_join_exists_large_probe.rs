use std::fs;
use std::process::Command;

#[test]
fn correlated_exists_rebinds_after_streaming_the_large_input() {
    let db = std::env::temp_dir().join(format!("rusthouse-exists-large-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE dimension (id UInt64)",
        "CREATE TABLE extra (id UInt64)",
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE lookup (id Nullable(UInt64))",
        "INSERT INTO dimension VALUES (1), (1), (2), (3)",
        "INSERT INTO extra VALUES (1), (2), (3)",
        "INSERT INTO lookup VALUES (1), (1), (3), (NULL)",
        "INSERT INTO facts SELECT number FROM numbers(40000)",
        "INSERT INTO facts SELECT number FROM numbers(40000, 40000)",
        "INSERT INTO facts SELECT number FROM numbers(80000, 40000)",
        "INSERT INTO facts SELECT number FROM numbers(120000, 40000)",
        "INSERT INTO facts SELECT number FROM numbers(160000, 40000)",
        "INSERT INTO facts SELECT number FROM numbers(200000, 40000)",
        "INSERT INTO facts SELECT number FROM numbers(240000, 25000)",
    ] {
        let out = sql(statement);
        assert!(
            out.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let query = "SELECT d.id, COUNT(*) AS n FROM dimension d JOIN extra e ON d.id = e.id JOIN facts f ON f.id = e.id WHERE EXISTS (SELECT * FROM lookup x WHERE x.id = f.id) GROUP BY d.id ORDER BY d.id";
    let out = sql(query);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"d.id\tn\n1\t2\n3\t1\n");
    let reversed = "SELECT d.id, COUNT(*) AS n FROM facts f JOIN extra e ON f.id = e.id JOIN dimension d ON d.id = e.id WHERE EXISTS (SELECT * FROM lookup x WHERE x.id = f.id) GROUP BY d.id ORDER BY d.id";
    assert_eq!(sql(reversed).stdout, out.stdout);
    let not = query.replace("WHERE EXISTS", "WHERE NOT EXISTS");
    assert_eq!(sql(&not).stdout, b"d.id\tn\n2\t1\n");
    assert_eq!(
        sql(&(query.to_owned() + " LIMIT 1 OFFSET 1")).stdout,
        b"d.id\tn\n3\t1\n"
    );
    assert!(
        !sql(&query.replace("x.id = f.id", "x.id = f.missing"))
            .status
            .success()
    );
    fs::remove_dir_all(db).unwrap();
}
