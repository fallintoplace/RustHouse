use std::fs;
use std::process::Command;

#[test]
fn ordered_inner_join_pages_full_large_fact_scan() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-ordered-stream-join-{}", std::process::id()
    ));
    let sql = |text: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", text, "--database"])
            .arg(&database)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimension (id UInt64)",
        "INSERT INTO dimension SELECT number FROM numbers(8192)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    for _ in 0..5 {
        let output = sql("INSERT INTO facts SELECT number FROM numbers(8192)");
        assert!(output.status.success(), "{output:?}");
    }
    let query = "SELECT f.id FROM facts f JOIN dimension d ON f.id = d.id ORDER BY f.id DESC LIMIT 4 OFFSET 3";
    let output = sql(query);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"f.id\n8191\n8191\n8190\n8190\n");
    fs::remove_dir_all(database).unwrap();
}
