use std::fs;
use std::process::Command;

#[test]
fn streaming_join_ignores_unreferenced_wide_dimension_payloads() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-streaming-dimension-pruning-{}",
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
        "CREATE TABLE fact (id UInt64, dimension_id UInt64)",
        "CREATE TABLE dimension (id UInt64, region_id UInt64, unused String)",
        "CREATE TABLE region (id UInt64, name String, unused String)",
        "INSERT INTO fact VALUES (1, 10), (2, 20)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let payload = "x".repeat(8192);
    let inserts = [
        format!(
            "INSERT INTO dimension VALUES (10, 100, '{payload}'), (20, 200, '{payload}')"
        ),
        format!(
            "INSERT INTO region VALUES (100, 'east', '{payload}'), (200, 'west', '{payload}')"
        ),
    ];
    for statement in inserts {
        let output = sql(&statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let result = sql(
        "SELECT f.id, r.name FROM fact f JOIN dimension d ON f.dimension_id = d.id JOIN region r ON d.region_id = r.id LIMIT 2",
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"f.id\tr.name\n1\teast\n2\twest\n");

    let wide = sql(
        "SELECT f.id, d.unused FROM fact f JOIN dimension d ON f.dimension_id = d.id JOIN region r ON d.region_id = r.id LIMIT 1",
    );
    assert!(wide.status.success(), "{wide:?}");
    assert_eq!(
        String::from_utf8(wide.stdout).unwrap(),
        format!("f.id\td.unused\n1\t{payload}\n")
    );
    fs::remove_dir_all(database).unwrap();
}
