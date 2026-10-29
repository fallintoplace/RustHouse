use std::fs;
use std::process::Command;

#[test]
fn streaming_chain_preserves_projection_and_join_keys_without_wide_fact_payloads() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-streaming-fact-pruning-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&database);
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&database)
            .output()
            .expect("run RustHouse")
    };
    for statement in [
        "CREATE TABLE fact (id UInt64, dimension_id UInt64, unused String)",
        "CREATE TABLE dimension (id UInt64, region_id UInt64)",
        "CREATE TABLE region (id UInt64, name String)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let payload = "x".repeat(8192);
    let inserts = [
        format!("INSERT INTO fact VALUES (1, 10, '{payload}'), (2, 20, '{payload}')"),
        "INSERT INTO dimension VALUES (10, 100), (20, 200)".to_owned(),
        "INSERT INTO region VALUES (100, 'east'), (200, 'west')".to_owned(),
    ];
    for statement in inserts {
        let output = sql(&statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let selected = sql(
        "SELECT f.id, r.name FROM fact f JOIN dimension d ON f.dimension_id = d.id JOIN region r ON d.region_id = r.id LIMIT 2",
    );
    assert!(selected.status.success(), "{selected:?}");
    assert_eq!(selected.stdout, b"f.id\tr.name\n1\teast\n2\twest\n");

    // An explicitly requested payload must still be decoded and emitted.
    let wide = sql(
        "SELECT f.id, f.unused FROM fact f JOIN dimension d ON f.dimension_id = d.id JOIN region r ON d.region_id = r.id LIMIT 1",
    );
    assert!(wide.status.success(), "{wide:?}");
    assert_eq!(
        String::from_utf8(wide.stdout).unwrap(),
        format!("f.id\tf.unused\n1\t{payload}\n")
    );
    fs::remove_dir_all(database).unwrap();
}
