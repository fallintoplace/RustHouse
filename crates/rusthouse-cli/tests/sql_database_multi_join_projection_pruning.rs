use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(statement: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .expect("run RustHouse")
}

#[test]
fn chained_joins_preserve_keys_and_outer_nulls_with_unprojected_wide_columns() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-chain-pruned-{}", std::process::id()
    ));
    let _ = fs::remove_dir_all(&database);
    for statement in [
        "CREATE TABLE fact (id UInt64, dimension_id UInt64, unused String)",
        "CREATE TABLE dimension (id UInt64, region_id UInt64, unused String)",
        "CREATE TABLE region (id UInt64, name String, unused String)",
    ] {
        let result = sql(statement, &database);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let payload = "x".repeat(8 * 1024);
    for statement in [
        format!("INSERT INTO fact VALUES (1, 10, '{payload}'), (2, 20, '{payload}')"),
        format!("INSERT INTO dimension VALUES (10, 100, '{payload}'), (20, 200, '{payload}')"),
        format!("INSERT INTO region VALUES (100, 'east', '{payload}')"),
    ] {
        let result = sql(&statement, &database);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = "SELECT f.id, r.name FROM fact f JOIN dimension d ON f.dimension_id = d.id LEFT JOIN region r ON d.region_id = r.id ORDER BY f.id";
    let result = sql(query, &database);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"f.id\tr.name\n1\teast\n2\t\\N\n");
    fs::remove_dir_all(database).unwrap();
}
