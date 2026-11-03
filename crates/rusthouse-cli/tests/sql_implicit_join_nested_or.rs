use std::fs;
use std::process::Command;

#[test]
fn implicit_join_preserves_nested_or_filters() {
    let db = std::env::temp_dir().join(format!("rusthouse-implicit-or-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE part (p_key UInt64)",
        "CREATE TABLE supplier (s_key UInt64)",
        "CREATE TABLE partsupp (ps_part UInt64, ps_supplier UInt64)",
        "CREATE TABLE nullable_filter (filter_id UInt64, flag Nullable(UInt64))",
        "INSERT INTO part VALUES (1), (2)",
        "INSERT INTO supplier VALUES (3), (4)",
        "INSERT INTO partsupp VALUES (1, 3), (1, 4), (2, 3)",
        "INSERT INTO nullable_filter VALUES (1, NULL), (2, 1)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let nested = "SELECT SUM(ps_part * 1) AS total FROM part, supplier, partsupp WHERE p_key = ps_part AND s_key = ps_supplier AND (ps_supplier = 3 OR ps_supplier = 4)";
    for (query, expected) in [
        (nested.to_owned(), "4"),
        (nested.replace("ps_supplier = 4", "ps_supplier = 99"), "3"),
        (nested.replace("ps_supplier = 3 OR ps_supplier = 4", "(ps_supplier = 3 AND ps_part = 2) OR ps_supplier = 4"), "3"),
        ("SELECT SUM(p_key * 1) AS total FROM part, nullable_filter WHERE p_key = filter_id AND (flag = 1 OR flag IS NULL)".into(), "3"),
    ] {
        let result = sql(&query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, format!("total\n{expected}\n").as_bytes());
    }
    let unsafe_or = "SELECT SUM(ps_part * 1) AS total FROM part, supplier, partsupp WHERE p_key = ps_part AND s_key = ps_supplier OR ps_supplier = 3";
    assert!(!sql(unsafe_or).status.success());
    fs::remove_dir_all(db).unwrap();
}
