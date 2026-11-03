use std::fs;
use std::process::Command;

#[test]
fn q14_calculates_ratio_of_conditional_and_total_sums() {
    let db = std::env::temp_dir().join(format!("rusthouse-tpch-q14-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE part (p_partkey UInt64, p_type String)",
        "CREATE TABLE lineitem (l_partkey UInt64, l_extendedprice Float64, l_discount Float64, l_shipdate Date)",
        "INSERT INTO part VALUES (1, 'PROMO X'), (2, 'OTHER')",
        "INSERT INTO lineitem VALUES (1, 100, 0.1, '1995-09-01'), (2, 75, 0.2, '1995-09-30'), (1, 999, 0, '1995-10-01')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = include_str!("../../../bench/tpch/queries/q14.sql");
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"promo_revenue\n60\n");
    let empty = "SELECT 100 * SUM(l_extendedprice * (1 - l_discount)) / SUM(l_discount * 1) AS ratio FROM lineitem l JOIN part p ON l.l_partkey = p.p_partkey WHERE l_shipdate < DATE '1970-01-01'";
    let result = sql(empty);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"ratio\n\\N\n");
    let limited = "SELECT SUM(l_extendedprice * 1) / SUM(l_discount * 1) AS ratio FROM lineitem l JOIN part p ON l.l_partkey = p.p_partkey LIMIT 0";
    assert_eq!(sql(limited).stdout, b"ratio\n");
    let offset = "SELECT SUM(l_extendedprice * 1) / SUM(l_discount * 1) AS ratio FROM lineitem l JOIN part p ON l.l_partkey = p.p_partkey OFFSET 1";
    assert_eq!(sql(offset).stdout, b"ratio\n");
    let zero = "SELECT SUM(l_extendedprice * 1) / SUM(l_discount * 0) AS ratio FROM lineitem l JOIN part p ON l.l_partkey = p.p_partkey";
    assert!(!sql(zero).status.success());
    let invalid = "SELECT SUM(l_extendedprice * 1) + SUM(missing * 1) AS ratio FROM lineitem l JOIN part p ON l.l_partkey = p.p_partkey LIMIT 0";
    assert!(!sql(invalid).status.success());
    let reserved = "SELECT SUM(l_extendedprice * 1) + __join_sum_0 AS ratio FROM lineitem l JOIN part p ON l.l_partkey = p.p_partkey";
    assert!(!sql(reserved).status.success());
    let result = sql("TRUNCATE TABLE part");
    assert!(result.status.success(), "{result:?}");
    let empty_build = "SELECT (1 / 0) + SUM(l_extendedprice * 1) AS total FROM lineitem l JOIN part p ON l.l_partkey = p.p_partkey";
    assert!(!sql(empty_build).status.success());
    fs::remove_dir_all(db).unwrap();
}
