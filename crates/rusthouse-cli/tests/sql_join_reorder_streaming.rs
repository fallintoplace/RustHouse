use std::fs;
use std::process::Command;

#[test]
fn grouped_inner_chain_streams_large_last_relation() {
    let db = std::env::temp_dir().join(format!("rusthouse-reorder-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE dimension (id UInt64, grp String)",
        "CREATE TABLE extra (id UInt64)",
        "CREATE TABLE facts (id UInt64)",
        "INSERT INTO dimension VALUES (1, 'one'), (1, 'one'), (2, 'two')",
        "INSERT INTO extra VALUES (1), (2)",
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
    let query = "SELECT d.grp, SUM(f.id) AS total FROM dimension d JOIN extra e ON d.id = e.id JOIN facts f ON f.id = d.id GROUP BY d.grp ORDER BY d.grp";
    let out = sql(query);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"d.grp\ttotal\none\t2\ntwo\t2\n");
    let reversed = "SELECT d.grp, SUM(f.id) AS total FROM facts f JOIN dimension d ON f.id = d.id JOIN extra e ON d.id = e.id GROUP BY d.grp ORDER BY d.grp";
    assert_eq!(sql(reversed).stdout, out.stdout);
    let multi_key = query.replace("f.id = d.id", "f.id = d.id AND f.id = e.id");
    assert_eq!(sql(&multi_key).stdout, out.stdout);
    fs::remove_dir_all(db).unwrap();
}
