use std::fs;
use std::process::Command;

#[test]
fn streamed_inner_joins_filter_typed_fact_and_dimension_columns() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-comparison-{}", std::process::id()
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (id UInt64, amount Nullable(Int64))",
        "CREATE TABLE dims (id UInt64, label String)",
        "INSERT INTO facts SELECT number FROM numbers(40000)",
        "INSERT INTO dims VALUES (39998, 'EU'), (39999, 'US')",
    ] {
        let actual = sql(statement);
        assert!(actual.status.success(), "{statement}: {actual:?}");
    }
    for (statement, expected) in [
        (
            "SELECT f.id, d.label FROM facts f JOIN dims d ON f.id = d.id WHERE f.id >= 39999 LIMIT 2",
            b"f.id\td.label\n39999\tUS\n".as_slice(),
        ),
        (
            "SELECT f.id, d.label FROM facts f JOIN dims d ON f.id = d.id WHERE d.label = 'EU' LIMIT 2",
            b"f.id\td.label\n39998\tEU\n",
        ),
        (
            "SELECT f.id FROM facts f JOIN dims d ON f.id = d.id WHERE d.label != 'EU' LIMIT 1",
            b"f.id\n39999\n",
        ),
        (
            "SELECT f.id FROM facts f JOIN dims d ON f.id = d.id WHERE f.id > 0 LIMIT 1 OFFSET 1",
            b"f.id\n39999\n",
        ),
        (
            "SELECT f.id FROM facts f JOIN dims d ON f.id = d.id WHERE f.id >= 39998 LIMIT 0",
            b"f.id\n",
        ),
    ] {
        let actual = sql(statement);
        assert!(actual.status.success(), "{statement}: {actual:?}");
        assert_eq!(actual.stdout, expected, "{statement}");
    }
    for invalid in [
        "SELECT f.id FROM facts f LEFT JOIN dims d ON f.id = d.id WHERE f.id > 10",
        "SELECT f.id FROM facts f JOIN dims d ON f.id = d.id WHERE x.id > 10",
    ] {
        assert!(!sql(invalid).status.success(), "{invalid}");
    }
    fs::remove_dir_all(db).unwrap();
}
