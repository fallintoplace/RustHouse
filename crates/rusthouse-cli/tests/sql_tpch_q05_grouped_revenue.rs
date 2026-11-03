use std::fs;
use std::process::Command;

#[test]
fn tpch_q05_groups_discounted_revenue_across_six_tables() {
    let db = std::env::temp_dir().join(format!("rusthouse-tpch-q05-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE customer (c_custkey UInt64, c_nationkey UInt64)",
        "CREATE TABLE orders (o_orderkey UInt64, o_custkey UInt64, o_orderdate Date)",
        "CREATE TABLE lineitem (l_orderkey UInt64, l_suppkey UInt64, l_extendedprice Float64, l_discount Float64)",
        "CREATE TABLE supplier (s_suppkey UInt64, s_nationkey UInt64)",
        "CREATE TABLE nation (n_nationkey UInt64, n_regionkey UInt64, n_name String)",
        "CREATE TABLE region (r_regionkey UInt64, r_name String)",
        "INSERT INTO customer VALUES (1, 10), (2, 20)",
        "INSERT INTO orders VALUES (100, 1, '1994-02-01'), (101, 2, '1994-03-01'), (102, 1, '1995-01-01')",
        "INSERT INTO lineitem VALUES (100, 11, 100, 0.1), (100, 11, 50, 0), (101, 22, 200, 0.25), (100, 22, 900, 0), (102, 11, 900, 0)",
        "INSERT INTO supplier VALUES (11, 10), (22, 20)",
        "INSERT INTO nation VALUES (10, 1, 'CHINA'), (20, 1, 'INDIA')",
        "INSERT INTO region VALUES (1, 'ASIA')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = include_str!("../../../bench/tpch/queries/q05.sql");
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"n_name\trevenue\nINDIA\t150\nCHINA\t140\n");
    fs::remove_dir_all(db).unwrap();
}
