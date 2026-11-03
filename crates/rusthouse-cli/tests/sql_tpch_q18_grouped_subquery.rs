use std::fs;
use std::process::Command;

#[test]
fn q18_filters_joined_rows_by_grouped_subquery() {
    let db = std::env::temp_dir().join(format!("rusthouse-tpch-q18-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE customer (c_custkey UInt64, c_name String)",
        "CREATE TABLE orders (o_orderkey UInt64, o_custkey UInt64, o_orderdate Date, o_totalprice Float64)",
        "CREATE TABLE lineitem (l_orderkey UInt64, l_quantity Nullable(Float64))",
        "INSERT INTO customer VALUES (1, 'Ada'), (2, 'Bob')",
        "INSERT INTO orders VALUES (101, 1, '1995-01-01', 200), (102, 2, '1995-01-02', 100)",
        "INSERT INTO lineitem VALUES (101, 150), (101, 151), (102, 100), (102, NULL)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = include_str!("../../../bench/tpch/queries/q18.sql");
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"c_name\tc_custkey\to_orderkey\to_orderdate\to_totalprice\tsum(l_quantity)\nAda\t1\t101\t1995-01-01\t200\t301\n");
    let empty = query.replace("sum(l_quantity) > 300", "sum(l_quantity) > 900");
    let result = sql(&empty);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"c_name\tc_custkey\to_orderkey\to_orderdate\to_totalprice\tsum(l_quantity)\n"
    );
    for statement in [
        "CREATE TABLE nullable_keys (l_orderkey Nullable(UInt64), l_quantity Float64)",
        "INSERT INTO nullable_keys VALUES (NULL, 900), (101, 301), (102, 100)",
        "CREATE TABLE wrong_keys (l_orderkey UInt32, l_quantity Float64)",
        "INSERT INTO wrong_keys VALUES (101, 301)",
        "CREATE TABLE many_keys (l_orderkey UInt64, l_quantity Float64)",
        "CREATE TABLE signed_left (ak Int64)",
        "CREATE TABLE signed_middle (bk Int64)",
        "CREATE TABLE signed_right (ck Int64, amount Int64)",
        "CREATE TABLE signed_inner (kk Nullable(Int64), v Int64)",
        "INSERT INTO signed_left VALUES (-1), (2)",
        "INSERT INTO signed_middle VALUES (-1), (2)",
        "INSERT INTO signed_right VALUES (-1, 4), (2, 6)",
        "INSERT INTO signed_inner VALUES (-1, 4), (NULL, 8), (2, 2)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let many_values = (101..121)
        .map(|key| format!("({key}, 301)"))
        .collect::<Vec<_>>()
        .join(", ");
    let result = sql(&format!("INSERT INTO many_keys VALUES {many_values}"));
    assert!(result.status.success(), "{result:?}");
    let nullable = query.replace(
        "FROM\n            lineitem",
        "FROM\n            nullable_keys",
    );
    let result = sql(&nullable);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"c_name\tc_custkey\to_orderkey\to_orderdate\to_totalprice\tsum(l_quantity)\nAda\t1\t101\t1995-01-01\t200\t301\n");
    let many = query.replace("FROM\n            lineitem", "FROM\n            many_keys");
    let result = sql(&many);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"c_name\tc_custkey\to_orderkey\to_orderdate\to_totalprice\tsum(l_quantity)\nAda\t1\t101\t1995-01-01\t200\t301\nBob\t2\t102\t1995-01-02\t100\t100\n");
    let mismatch = query.replace("FROM\n            lineitem", "FROM\n            wrong_keys");
    assert!(!sql(&mismatch).status.success());
    let extra = query.replace(
        "AND c_custkey = o_custkey",
        "AND o_totalprice > 150 AND c_custkey = o_custkey",
    );
    let result = sql(&extra);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"c_name\tc_custkey\to_orderkey\to_orderdate\to_totalprice\tsum(l_quantity)\nAda\t1\t101\t1995-01-01\t200\t301\n");
    let signed = "SELECT ak, SUM(amount) AS total FROM signed_left a JOIN signed_middle b ON a.ak = b.bk JOIN signed_right c ON b.bk = c.ck WHERE ak IN (SELECT kk FROM signed_inner GROUP BY kk HAVING SUM(v) > 3) GROUP BY ak ORDER BY ak";
    let result = sql(signed);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"ak\ttotal\n-1\t4\n");
    let correlated = query.replace(
        "GROUP BY\n            l_orderkey",
        "WHERE l_orderkey = o_orderkey GROUP BY l_orderkey",
    );
    assert!(!sql(&correlated).status.success());
    fs::remove_dir_all(db).unwrap();
}
