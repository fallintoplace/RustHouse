use std::fs;
use std::process::Command;

#[test]
fn streaming_ordered_join_keeps_sort_only_columns_on_build_side() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-ordered-pruned-{}",
        std::process::id()
    ));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"]).arg(&db).output().unwrap()
    };
    for query in [
        "CREATE TABLE facts (id UInt64)",
        "CREATE TABLE dimension (id UInt64, score UInt64, payload String)",
        "INSERT INTO dimension VALUES (1, 70, 'unused'), (2, 90, 'unused'), (3, 80, 'unused')",
        "INSERT INTO facts SELECT number FROM numbers(40000)",
    ] {
        let out = sql(query);
        assert!(out.status.success(), "{query}: {out:?}");
    }
    let out = sql("SELECT f.id FROM facts f JOIN dimension d ON f.id = d.id ORDER BY d.score DESC LIMIT 2");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(out.stdout, b"f.id\n2\n3\n");
    fs::remove_dir_all(db).unwrap();
}
