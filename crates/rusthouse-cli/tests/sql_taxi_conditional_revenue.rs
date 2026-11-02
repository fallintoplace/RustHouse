use std::fs;
use std::process::Command;

#[test]
fn taxi_revenue_supports_two_conditional_sum_metrics_in_one_select() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-taxi-two-case-revenue-{}",
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
        "CREATE TABLE trips (fare_cents UInt64, tip_cents UInt64, card UInt8)",
        "INSERT INTO trips VALUES (1000, 100, 1), (2000, 0, 0), (3000, 300, 1), (4000, 400, 0)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = "SELECT SUM(CASE WHEN card = 1 THEN fare_cents ELSE 0 END) AS card_revenue, SUM(CASE WHEN card = 0 THEN tip_cents ELSE 0 END) AS cash_tips, SUM(fare_cents) AS all_revenue, COUNT(*) AS rides FROM trips";
    let output = sql(query);
    assert!(output.status.success(), "{query}: {output:?}");
    assert_eq!(
        output.stdout,
        b"card_revenue\tcash_tips\tall_revenue\trides\n4000\t400\t10000\t4\n"
    );
    let mixed = "SELECT SUM(CASE WHEN card = 1 THEN fare_cents ELSE -1 END), SUM(CASE WHEN card = 0 THEN tip_cents ELSE 0 END) FROM trips";
    let output = sql(mixed);
    assert!(output.status.success(), "{mixed}: {output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().lines().last(),
        Some("3998\t400"),
    );
    fs::remove_dir_all(db).unwrap();
}
