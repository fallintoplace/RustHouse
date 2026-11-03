use std::fs;
use std::process::Command;

#[test]
fn tpch_q10_accepts_reordered_group_keys() {
    let db = std::env::temp_dir().join(format!("rusthouse-tpch-q10-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE customer (c_custkey UInt64, c_name String, c_acctbal Float64, c_nationkey UInt64, c_address String, c_phone String, c_comment String)",
        "CREATE TABLE orders (o_orderkey UInt64, o_custkey UInt64, o_orderdate Date)",
        "CREATE TABLE lineitem (l_orderkey UInt64, l_extendedprice Float64, l_discount Float64, l_returnflag String)",
        "CREATE TABLE nation (n_nationkey UInt64, n_name String)",
        "INSERT INTO customer VALUES (1, 'Ada', 5, 10, 'A', 'P1', 'C1'), (2, 'Bob', 6, 20, 'B', 'P2', 'C2')",
        "INSERT INTO orders VALUES (100, 1, '1993-10-10'), (101, 2, '1993-11-11')",
        "INSERT INTO lineitem VALUES (100, 100, 0.1, 'R'), (101, 200, 0.25, 'R'), (101, 999, 0, 'N')",
        "INSERT INTO nation VALUES (10, 'CHINA'), (20, 'INDIA')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = include_str!("../../../bench/tpch/queries/q10.sql");
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"c_custkey\tc_name\trevenue\tc_acctbal\tn_name\tc_address\tc_phone\tc_comment\n2\tBob\t150\t6\tINDIA\tB\tP2\tC2\n1\tAda\t90\t5\tCHINA\tA\tP1\tC1\n");
    fs::remove_dir_all(db).unwrap();
}
