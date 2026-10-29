use std::fs;
use std::process::Command;

#[test]
fn count_over_chained_join_does_not_materialize_hundreds_of_thousands_of_matches() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-streaming-chain-count-{}", std::process::id()
    ));
    let _ = fs::remove_dir_all(&db);
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE fact (id UInt64, dimension_id UInt64)",
        "CREATE TABLE dimension (id UInt64, region_id UInt64)",
        "CREATE TABLE region (id UInt64, label String)",
        "INSERT INTO dimension VALUES (10, 20)",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let fact = (0..200).map(|id| format!("({id},10)")).collect::<Vec<_>>().join(",");
    let region = (0..700).map(|_| "(20,'yes')").collect::<Vec<_>>().join(",");
    for statement in [
        format!("INSERT INTO fact VALUES {fact}"),
        format!("INSERT INTO region VALUES {region}"),
    ] {
        let output = sql(&statement);
        assert!(output.status.success(), "{output:?}");
    }
    let result = sql("SELECT COUNT(*) FROM fact f JOIN dimension d ON f.dimension_id = d.id JOIN region r ON d.region_id = r.id");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"count\n140000\n");
    let filtered = sql("SELECT COUNT(*) FROM fact f JOIN dimension d ON f.dimension_id = d.id LEFT JOIN region r ON d.region_id = r.id WHERE r.label IS NULL");
    assert!(filtered.status.success(), "{filtered:?}");
    assert_eq!(filtered.stdout, b"count\n0\n");
    fs::remove_dir_all(db).unwrap();
}
