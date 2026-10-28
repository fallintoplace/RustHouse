use std::fs;
use std::process::Command;

#[test]
fn using_star_projects_shared_keys_first_in_using_list_order() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-using-order-{}",
        std::process::id()
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE lefts (payload String, id UInt64, region UInt64)",
        "CREATE TABLE rights (label String, region UInt64, id UInt64)",
        "INSERT INTO lefts VALUES ('a', 1, 10)",
        "INSERT INTO rights VALUES ('right', 20, 2)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT * FROM lefts l FULL JOIN rights r USING (region, id) LIMIT 10",
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"l.region\tl.id\tl.payload\tr.label\n10\t1\ta\t\\N\n20\t2\t\\N\tright\n"
    );
    fs::remove_dir_all(db).unwrap();
}
