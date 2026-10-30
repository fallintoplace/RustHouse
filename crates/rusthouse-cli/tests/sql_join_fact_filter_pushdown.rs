use std::fs;
use std::process::Command;

#[test]
fn fact_predicate_is_safe_before_any_join_probe_and_offset() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-fact-filter-probes-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&database);
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&database)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64, quality UInt64)",
        "CREATE TABLE dims (id UInt64, name String)",
        "INSERT INTO facts VALUES (1, 0), (1, 1), (2, 1), (3, 0)",
        "INSERT INTO dims VALUES (1, 'first'), (1, 'second'), (2, 'third')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT f.id, d.name FROM facts f ANY INNER JOIN dims d ON f.id = d.id WHERE f.quality = 1",
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"f.id\td.name\n1\tfirst\n2\tthird\n");

    let result = sql(
        "SELECT f.id FROM facts f JOIN dims d ON f.id = d.id WHERE f.quality = 1 LIMIT 1 OFFSET 1",
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"f.id\n1\n");
    fs::remove_dir_all(database).unwrap();
}
