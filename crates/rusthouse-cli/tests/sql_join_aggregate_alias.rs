use std::fs;
use std::process::Command;

#[test]
fn join_scalar_aggregates_use_explicit_output_aliases() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-join-aggregate-alias-{}",
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
        "CREATE TABLE trips (id UInt64, amount Int64)",
        "CREATE TABLE zones (id UInt64)",
        "CREATE TABLE boroughs (id UInt64)",
        "INSERT INTO trips VALUES (1, 10), (2, 20), (2, 30)",
        "INSERT INTO zones VALUES (1), (2), (2)",
        "INSERT INTO boroughs VALUES (2)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT COUNT(*) AS rides FROM trips t JOIN zones z ON t.id = z.id",
            "rides\n5\n",
        ),
        (
            "SELECT COUNT(t.amount) AS paid FROM trips t JOIN zones z ON t.id = z.id",
            "paid\n5\n",
        ),
        (
            "SELECT SUM(t.amount) AS revenue FROM trips t JOIN zones z ON t.id = z.id",
            "revenue\n110\n",
        ),
        (
            "SELECT AVG(t.amount) AS mean_fare FROM trips t JOIN zones z ON t.id = z.id",
            "mean_fare\n22\n",
        ),
        (
            "SELECT COUNT(*) AS matched FROM trips t JOIN zones z ON t.id = z.id JOIN boroughs b ON z.id = b.id",
            "matched\n4\n",
        ),
        (
            "SELECT COUNT(*) AS rides FROM trips t JOIN zones z ON t.id = z.id LIMIT 0",
            "rides\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
