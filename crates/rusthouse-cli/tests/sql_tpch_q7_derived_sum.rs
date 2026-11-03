use std::fs;
use std::process::Command;

#[test]
fn q7_groups_bilateral_shipping_revenue() {
    let db = std::env::temp_dir().join(format!("rusthouse-q7-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE supplier (s_suppkey UInt64, s_nationkey UInt64)",
        "CREATE TABLE lineitem (l_suppkey UInt64, l_orderkey UInt64, l_shipdate Date, l_extendedprice Float64, l_discount Float64)",
        "CREATE TABLE orders (o_orderkey UInt64, o_custkey UInt64)",
        "CREATE TABLE customer (c_custkey UInt64, c_nationkey UInt64)",
        "CREATE TABLE nation (n_nationkey UInt64, n_name String)",
        "INSERT INTO nation VALUES (1, 'FRANCE'), (2, 'GERMANY'), (3, 'USA')",
        "INSERT INTO supplier VALUES (11, 1), (22, 2), (33, 3)",
        "INSERT INTO customer VALUES (101, 1), (102, 2), (103, 3)",
        "INSERT INTO orders VALUES (1, 101), (2, 102), (3, 103)",
        "INSERT INTO lineitem VALUES (11, 2, '1995-01-01', 100, 0.1), (11, 2, '1995-12-31', 10, 0), (22, 1, '1996-12-31', 80, 0.25), (11, 2, '1997-01-01', 999, 0), (11, 3, '1995-06-01', 999, 0), (33, 2, '1995-06-01', 999, 0)",
    ] {
        let result = sql(statement);
        assert!(
            result.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let query = include_str!("../../../bench/tpch/queries/q07.sql");
    let result = sql(query);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"supp_nation\tcust_nation\tl_year\trevenue\nFRANCE\tGERMANY\t1995\t100\nGERMANY\tFRANCE\t1996\t60\n");
    let reordered = query.replace(
        "GROUP BY\n    supp_nation,\n    cust_nation,\n    l_year",
        "GROUP BY\n    l_year,\n    supp_nation,\n    cust_nation",
    );
    let reordered_result = sql(&reordered);
    assert!(
        reordered_result.status.success(),
        "{}",
        String::from_utf8_lossy(&reordered_result.stderr)
    );
    assert_eq!(reordered_result.stdout, result.stdout);
    let zero = query.trim_end().trim_end_matches(';').to_owned() + " LIMIT 0";
    let zero_result = sql(&zero);
    assert!(
        zero_result.status.success(),
        "{}",
        String::from_utf8_lossy(&zero_result.stderr)
    );
    assert_eq!(
        zero_result.stdout,
        b"supp_nation\tcust_nation\tl_year\trevenue\n"
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
        b"supp_nation\tcust_nation\tl_year\trevenue\nGERMANY\tFRANCE\t1996\t60\n"
    );
    for invalid in [
        query.replace(
            "cust_nation,\n    l_year\nORDER BY",
            "supp_nation,\n    l_year\nORDER BY",
        ),
        query.replace("n2.n_name AS cust_nation", "n2.n_name AS wrong"),
        query.replace("l_shipdate BETWEEN", "missing BETWEEN"),
        query.replace("ORDER BY\n    supp_nation,", "ORDER BY\n    s_suppkey,"),
    ] {
        assert!(!sql(&invalid).status.success(), "{invalid}");
    }
    fs::remove_dir_all(db).unwrap();
}
