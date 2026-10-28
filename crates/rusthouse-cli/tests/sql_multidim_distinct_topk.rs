use std::fs;
use std::process::Command;

#[test]
fn group_distinct_topk_ranks_full_aggregates_and_keeps_stable_ties() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-multidim-distinct-topk-{}",
        std::process::id()
    ));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE facts (region UInt8, segment UInt8, visitor UInt8)",
        "INSERT INTO facts VALUES (1, 1, 1), (1, 2, 1), (2, 1, 1), (2, 2, 1), (1, 1, 2)",
        "INSERT INTO facts VALUES (1, 1, 3), (1, 2, 2), (2, 1, 2), (2, 2, 1), (1, 1, 1)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(
        "SELECT region, segment, count(DISTINCT visitor) FROM facts GROUP BY region, segment ORDER BY count DESC LIMIT 2 OFFSET 1"
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"region\tsegment\tcountDistinct(visitor)\n1\t2\t2\n2\t1\t2\n"
    );
    fs::remove_dir_all(db).unwrap();
}
