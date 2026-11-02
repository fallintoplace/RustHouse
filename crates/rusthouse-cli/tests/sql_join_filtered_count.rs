use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(query: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn filtered_inner_join_counts_preserve_duplicates_nulls_and_any_first_match() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-filter-count-{}", std::process::id()));
    for statement in [
        "CREATE TABLE facts (id UInt64, amount Nullable(Int64), active UInt8)",
        "CREATE TABLE dims (id UInt64, zone String)",
        "INSERT INTO facts VALUES (1, 10, 1), (1, 20, 0), (2, 5, 1), (3, NULL, 1), (4, 7, 1)",
        "INSERT INTO dims VALUES (1, 'airport'), (1, 'other'), (2, 'airport'), (3, 'airport'), (5, 'airport')",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let cases: &[(&str, &[u8])] = &[
        (
            "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE f.active = 1",
            b"count\n4\n",
        ),
        (
            "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE d.zone = 'airport'",
            b"count\n4\n",
        ),
        (
            "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE f.amount > 9",
            b"count\n4\n",
        ),
        (
            "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE f.amount > 100",
            b"count\n0\n",
        ),
        (
            "SELECT COUNT(*) FROM facts f ANY INNER JOIN dims d ON f.id = d.id WHERE d.zone = 'other'",
            b"count\n0\n",
        ),
        (
            "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE d.zone = 'airport' LIMIT 1",
            b"count\n4\n",
        ),
        (
            "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE f.active = 1 LIMIT 0",
            b"count\n",
        ),
        (
            "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE f.active = 1 OFFSET 1",
            b"count\n",
        ),
    ];
    for &(statement, expected) in cases {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
        assert_eq!(result.stdout, expected, "{statement}");
    }
    for statement in [
        "SELECT SUM(f.amount) FROM facts f JOIN dims d ON f.id = d.id WHERE f.active = 1",
        "SELECT COUNT(*) FROM facts f LEFT JOIN dims d ON f.id = d.id WHERE f.active = 1",
    ] {
        assert!(!sql(statement, &db).status.success(), "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}

#[test]
fn filtered_join_count_is_not_limited_to_materialized_result_rows() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-filter-large-{}", std::process::id()));
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dims (id UInt64)",
        "INSERT INTO facts SELECT number FROM numbers(135000)",
        "INSERT INTO dims SELECT number FROM numbers(135000)",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let statement = "SELECT COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE f.id >= 0";
    let result = sql(statement, &db);
    assert!(result.status.success(), "{statement}: {result:?}");
    assert_eq!(result.stdout, b"count\n135000\n");
    fs::remove_dir_all(db).unwrap();
}
