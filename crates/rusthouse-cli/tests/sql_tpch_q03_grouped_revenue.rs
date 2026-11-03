use std::fs;
use std::process::Command;

#[test]
fn tpch_q03_keeps_revenue_between_group_keys() {
    let db = std::env::temp_dir().join(format!("rusthouse-tpch-q03-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE customer (c_custkey UInt64, c_mktsegment String)",
        "CREATE TABLE orders (o_orderkey UInt64, o_custkey UInt64, o_orderdate Date, o_shippriority Int64)",
        "CREATE TABLE lineitem (l_orderkey UInt64, l_extendedprice Float64, l_discount Float64, l_shipdate Date)",
        "INSERT INTO customer VALUES (1, 'BUILDING'), (2, 'MACHINERY')",
        "INSERT INTO orders VALUES (10, 1, '1995-03-10', 0), (11, 1, '1995-03-01', 1), (12, 2, '1995-03-01', 0)",
        "INSERT INTO lineitem VALUES (10, 100, 0.1, '1995-03-20'), (11, 200, 0.25, '1995-03-25'), (12, 900, 0, '1995-03-25')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = include_str!("../../../bench/tpch/queries/q03.sql");
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"l_orderkey\trevenue\to_orderdate\to_shippriority\n11\t150\t1995-03-01\t1\n10\t90\t1995-03-10\t0\n");
    fs::remove_dir_all(db).unwrap();
}
