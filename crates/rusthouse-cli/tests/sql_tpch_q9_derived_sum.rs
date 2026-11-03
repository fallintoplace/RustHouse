use std::fs;
use std::process::Command;

#[test]
fn q9_groups_profit_from_a_derived_join() {
    let db = std::env::temp_dir().join(format!("rusthouse-q9-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE part (p_partkey UInt64, p_name String)",
        "CREATE TABLE supplier (s_suppkey UInt64, s_nationkey UInt64)",
        "CREATE TABLE lineitem (l_suppkey UInt64, l_partkey UInt64, l_orderkey UInt64, l_extendedprice Float64, l_discount Float64, l_quantity Float64)",
        "CREATE TABLE partsupp (ps_suppkey UInt64, ps_partkey UInt64, ps_supplycost Float64)",
        "CREATE TABLE orders (o_orderkey UInt64, o_orderdate Date)",
        "CREATE TABLE nation (n_nationkey UInt64, n_name String)",
        "INSERT INTO part VALUES (1, 'green part'), (2, 'red part'), (3, 'dark green')",
        "INSERT INTO supplier VALUES (10, 100), (20, 200)",
        "INSERT INTO nation VALUES (100, 'USA'), (200, 'CANADA')",
        "INSERT INTO partsupp VALUES (10, 1, 20), (10, 2, 1), (20, 3, 10)",
        "INSERT INTO orders VALUES (1000, '1995-01-01'), (1001, '1995-12-31'), (1002, '1994-02-01')",
        "INSERT INTO lineitem VALUES (10, 1, 1000, 100, 0.1, 2), (10, 1, 1001, 50, 0, 1), (10, 2, 1000, 999, 0, 1), (20, 3, 1002, 70, 0, 2)",
    ] {
        let out = sql(statement);
        assert!(
            out.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let query = include_str!("../../../bench/tpch/queries/q09.sql");
    let result = sql(query);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        result.stdout,
        b"nation\to_year\tsum_profit\nCANADA\t1994\t50\nUSA\t1995\t80\n"
    );
    let limited = query.trim_end().trim_end_matches(';').to_owned() + " LIMIT 1 OFFSET 1";
    let result = sql(&limited);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        result.stdout,
        b"nation\to_year\tsum_profit\nUSA\t1995\t80\n"
    );
    for invalid in [
        query.replace(
            "GROUP BY\n    nation,\n    o_year",
            "GROUP BY\n    nation,\n    amount",
        ),
        query.replace("n_name AS nation", "n_name AS wrong"),
        query.replace("p_name LIKE '%green%'", "missing LIKE '%green%'"),
        query.replace("ORDER BY\n    nation,", "ORDER BY\n    p_partkey,"),
        query.replace(
            "AS profit\nGROUP BY",
            "AS profit WHERE nation = 'USA' GROUP BY",
        ),
    ] {
        assert!(!sql(&invalid).status.success(), "{invalid}");
    }
    fs::remove_dir_all(db).unwrap();
}
