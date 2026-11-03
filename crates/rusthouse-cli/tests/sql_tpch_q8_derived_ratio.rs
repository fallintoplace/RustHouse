use std::fs;
use std::process::Command;

#[test]
fn q8_calculates_national_market_share() {
    let db = std::env::temp_dir().join(format!("rusthouse-q8-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE part (p_partkey UInt64, p_type String)",
        "CREATE TABLE supplier (s_suppkey UInt64, s_nationkey UInt64)",
        "CREATE TABLE lineitem (l_partkey UInt64, l_suppkey UInt64, l_orderkey UInt64, l_extendedprice Float64, l_discount Float64)",
        "CREATE TABLE orders (o_orderkey UInt64, o_custkey UInt64, o_orderdate Date)",
        "CREATE TABLE customer (c_custkey UInt64, c_nationkey UInt64)",
        "CREATE TABLE nation (n_nationkey UInt64, n_regionkey UInt64, n_name String)",
        "CREATE TABLE region (r_regionkey UInt64, r_name String)",
        "INSERT INTO part VALUES (1, 'ECONOMY ANODIZED STEEL'), (2, 'OTHER')",
        "INSERT INTO region VALUES (1, 'AMERICA'), (2, 'EUROPE')",
        "INSERT INTO nation VALUES (1, 1, 'BRAZIL'), (2, 1, 'USA'), (3, 2, 'FRANCE')",
        "INSERT INTO supplier VALUES (1, 1), (2, 2)",
        "INSERT INTO customer VALUES (11, 2), (12, 3)",
        "INSERT INTO orders VALUES (1, 11, '1995-01-01'), (2, 11, '1996-12-31'), (3, 12, '1995-06-01')",
        "INSERT INTO lineitem VALUES (1, 1, 1, 75, 0), (1, 2, 1, 25, 0), (1, 2, 2, 20, 0), (2, 1, 1, 999, 0), (1, 1, 3, 999, 0)",
    ] {
        let result = sql(statement);
        assert!(
            result.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let query = include_str!("../../../bench/tpch/queries/q08.sql");
    let result = sql(query);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"o_year\tmkt_share\n1995\t0.75\n1996\t0\n");
    let limited = query.trim_end().trim_end_matches(';').to_owned() + " LIMIT 1 OFFSET 1";
    let result = sql(&limited);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"o_year\tmkt_share\n1996\t0\n");
    for invalid in [
        query.replace("GROUP BY\n    o_year", "GROUP BY\n    nation"),
        query.replace("n2.n_name AS nation", "n2.n_name AS wrong"),
        query.replace("p_type =", "missing ="),
        query.replace("ORDER BY\n    o_year", "ORDER BY\n    nation"),
        query.replace(
            "AS all_nations\nGROUP BY",
            "AS all_nations WHERE nation = 'USA' GROUP BY",
        ),
    ] {
        assert!(!sql(&invalid).status.success(), "{invalid}");
    }
    fs::remove_dir_all(db).unwrap();
}
