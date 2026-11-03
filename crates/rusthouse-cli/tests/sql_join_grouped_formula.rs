use std::fs;
use std::process::Command;

#[test]
fn grouped_join_aggregate_formulas_handle_case_and_nulls() {
    let db = std::env::temp_dir().join(format!("rusthouse-group-formula-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE f (fk UInt64, amount Nullable(Float64))",
        "CREATE TABLE d (id UInt64, ek UInt64, grp String, kind Nullable(String))",
        "CREATE TABLE e (id UInt64, kind String)",
        "INSERT INTO f VALUES (1, 10), (2, 10), (3, NULL), (4, 5)",
        "INSERT INTO d VALUES (1, 10, 'A', 'b'), (2, 10, 'A', 'x'), (3, 10, 'C', 'b'), (4, 10, 'B', NULL)",
        "INSERT INTO e VALUES (10, 'other')",
    ] {
        let result = sql(statement);
        assert!(
            result.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let query = "SELECT d.grp, SUM(CASE WHEN d.kind = 'b' THEN f.amount ELSE 0 END) / SUM(f.amount) AS ratio FROM f JOIN d ON f.fk = d.id JOIN e ON d.ek = e.id GROUP BY d.grp ORDER BY d.grp";
    let result = sql(query);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"d.grp\tratio\nA\t0.5\nB\t0\nC\t\\N\n");
    let two_tables = query.replace(" JOIN e ON d.ek = e.id", "");
    let two_result = sql(&two_tables);
    assert!(
        two_result.status.success(),
        "{}",
        String::from_utf8_lossy(&two_result.stderr)
    );
    assert_eq!(two_result.stdout, result.stdout);
    let scalar = "SELECT SUM(CASE WHEN d.kind = 'b' THEN f.amount ELSE 0 END) / SUM(f.amount) AS ratio FROM f JOIN d ON f.fk = d.id";
    let scalar_result = sql(scalar);
    assert!(
        scalar_result.status.success(),
        "{}",
        String::from_utf8_lossy(&scalar_result.stderr)
    );
    assert_eq!(scalar_result.stdout, b"ratio\n0.4\n");
    let ordered = query.replace(
        "ORDER BY d.grp",
        "ORDER BY ratio DESC NULLS LAST LIMIT 1 OFFSET 1",
    );
    let result = sql(&ordered);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"d.grp\tratio\nB\t0\n");
    for invalid in [
        query.replace("d.kind = 'b'", "kind = 'b'"),
        query.replace("d.kind = 'b'", "d.id = 'b'"),
        query.replace("f.amount ELSE", "f.missing ELSE") + " LIMIT 0",
        query.replace("JOIN d ON", "LEFT JOIN d ON"),
        query.replace("SUM(f.amount)", "SUM(f.amount - f.amount)"),
    ] {
        assert!(!sql(&invalid).status.success(), "{invalid}");
    }
    fs::remove_dir_all(db).unwrap();
}
