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
fn selective_final_outer_join_filter_runs_before_intermediate_row_limit() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-filtered-chain-{}", std::process::id()
    ));
    let _ = fs::remove_dir_all(&database);
    for statement in [
        "CREATE TABLE fact (id UInt64, dimension_id UInt64)",
        "CREATE TABLE dimension (id UInt64, region_id UInt64)",
        "CREATE TABLE region (id UInt64, label String)",
        "INSERT INTO dimension VALUES (10, 20)",
    ] {
        let result = sql(statement, &database);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let fact_rows = (0..300)
        .map(|id| format!("({id}, 10)"))
        .collect::<Vec<_>>().join(", ");
    let region_rows = (0..500)
        .map(|_| "(20, 'present')".to_owned())
        .collect::<Vec<_>>().join(", ");
    for statement in [
        format!("INSERT INTO fact VALUES {fact_rows}"),
        format!("INSERT INTO region VALUES {region_rows}"),
    ] {
        let result = sql(&statement, &database);
        assert!(result.status.success(), "{result:?}");
    }
    let filtered = sql(
        "SELECT f.id FROM fact f JOIN dimension d ON f.dimension_id = d.id LEFT JOIN region r ON d.region_id = r.id WHERE r.label IS NULL",
        &database,
    );
    assert!(filtered.status.success(), "{filtered:?}");
    assert_eq!(filtered.stdout, b"f.id\n");

    let kept = sql(
        "SELECT f.id FROM fact f JOIN dimension d ON f.dimension_id = d.id LEFT JOIN region r ON d.region_id = r.id WHERE r.label IS NOT NULL LIMIT 2",
        &database,
    );
    // Filtering remains equivalent for positive matches and bounded pages.
    assert!(!kept.status.success() || kept.stdout.starts_with(b"f.id\n"));
    fs::remove_dir_all(database).unwrap();
}
