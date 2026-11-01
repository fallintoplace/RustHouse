use std::fs;
use std::process::Command;

#[test]
fn chained_join_with_only_null_build_keys_skips_all_fact_matches() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-null-only-hash-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
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
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE nullable_dim (id Nullable(UInt64))",
        "CREATE TABLE regions (id UInt64)",
        "INSERT INTO facts SELECT number FROM numbers(40000)",
        "INSERT INTO nullable_dim VALUES (NULL), (NULL)",
        "INSERT INTO regions VALUES (39999)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }

    let query = "SELECT f.id FROM facts f JOIN nullable_dim d ON f.id = d.id JOIN regions r ON d.id = r.id";
    let empty = sql(query);
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"f.id\n");

    let insert = sql("INSERT INTO nullable_dim VALUES (39999)");
    assert!(insert.status.success(), "{insert:?}");
    let matched = sql(query);
    assert!(matched.status.success(), "{matched:?}");
    assert_eq!(matched.stdout, b"f.id\n39999\n");
    fs::remove_dir_all(database).unwrap();
}
