use std::fs;
use std::process::Command;

#[test]
fn stream_grouped_count_on_large_fact_and_nullable_dimension() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-grouped-join-{}", std::process::id()
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dims (id UInt64, region Nullable(String))",
        "INSERT INTO dims VALUES (39998, 'EU'), (39999, NULL)",
        "INSERT INTO facts SELECT number FROM numbers(40000)",
        "INSERT INTO facts SELECT number FROM numbers(40000)",
    ] {
        let actual = sql(statement);
        assert!(actual.status.success(), "{statement}: {actual:?}");
    }
    for (statement, expected) in [
        (
            "SELECT d.region, COUNT(*) FROM facts f JOIN dims d ON f.id = d.id GROUP BY d.region ORDER BY d.region ASC NULLS LAST",
            b"d.region\tcount\nEU\t2\n\\N\t2\n".as_slice(),
        ),
        (
            "SELECT d.region, COUNT(*) FROM facts f JOIN dims d ON f.id = d.id GROUP BY d.region ORDER BY d.region ASC NULLS LAST LIMIT 1 OFFSET 1",
            b"d.region\tcount\n\\N\t2\n",
        ),
        (
            "SELECT d.region, COUNT(*) FROM facts f JOIN dims d ON f.id = d.id WHERE d.region IS NOT NULL GROUP BY d.region",
            b"d.region\tcount\nEU\t2\n",
        ),
        (
            "SELECT d.region, COUNT(*) FROM facts f JOIN dims d ON f.id = d.id GROUP BY d.region LIMIT 0",
            b"d.region\tcount\n",
        ),
    ] {
        let actual = sql(statement);
        assert!(actual.status.success(), "{statement}: {actual:?}");
        assert_eq!(actual.stdout, expected, "{statement}");
    }
    for invalid in [
        "SELECT d.region, COUNT(*) FROM facts f JOIN dims d ON f.id = d.id GROUP BY f.id",
        "SELECT d.region FROM facts f JOIN dims d ON f.id = d.id GROUP BY d.region",
    ] {
        assert!(!sql(invalid).status.success(), "{invalid}");
    }
    fs::remove_dir_all(db).unwrap();
}
