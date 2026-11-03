use std::fs;
use std::process::Command;

#[test]
fn grouped_case_streams_large_right_input_without_changing_results() {
    let db = std::env::temp_dir().join(format!("rusthouse-case-large-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE dimension (id UInt64, status Nullable(String))",
        "CREATE TABLE facts (id UInt64)",
        "INSERT INTO dimension VALUES (1, 'yes'), (1, 'yes'), (2, NULL), (3, 'no')",
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
    let query = "SELECT f.id, SUM(CASE WHEN status = 'yes' THEN 1 ELSE 0 END) AS n FROM dimension d JOIN facts f ON d.id = f.id GROUP BY f.id ORDER BY f.id";
    let out = sql(query);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"f.id\tn\n1\t2\n2\t0\n3\t0\n");
    let reversed = "SELECT f.id, SUM(CASE WHEN status = 'yes' THEN 1 ELSE 0 END) AS n FROM facts f JOIN dimension d ON d.id = f.id GROUP BY f.id ORDER BY f.id";
    assert_eq!(sql(reversed).stdout, out.stdout);
    assert_eq!(
        sql(&(query.to_owned() + " LIMIT 1 OFFSET 1")).stdout,
        b"f.id\tn\n2\t0\n"
    );
    assert_eq!(sql(&(query.to_owned() + " LIMIT 0")).stdout, b"f.id\tn\n");
    let any = query.replace("JOIN facts", "ANY INNER JOIN facts");
    assert!(!sql(&any).status.success());
    fs::remove_dir_all(db).unwrap();
}
