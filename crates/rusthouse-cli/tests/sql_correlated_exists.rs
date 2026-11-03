use std::fs;
use std::process::{Command, Output};

#[test]
fn correlated_exists_counts_each_outer_row_once_and_skips_nulls() {
    let db = std::env::temp_dir().join(format!("rusthouse-exists-{}", std::process::id()));
    let sql = |statement: &str| -> Output {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE orders (o_orderkey Nullable(UInt64), o_orderdate Date, o_orderpriority String)",
        "CREATE TABLE lineitem (l_orderkey Nullable(UInt64), l_commitdate Date, l_receiptdate Date)",
        "INSERT INTO orders VALUES (1, '1993-07-02', 'A'), (2, '1993-07-02', 'A'), (3, '1993-07-02', 'B'), (NULL, '1993-07-02', 'B'), (4, '1994-01-01', 'A')",
        "INSERT INTO lineitem VALUES (1, '1993-01-01', '1993-01-02'), (1, '1993-01-01', '1993-01-03'), (2, '1993-01-03', '1993-01-01'), (NULL, '1993-01-01', '1993-01-02'), (4, '1993-01-01', '1993-01-02')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let result = sql(include_str!("../../../bench/tpch/queries/q04.sql"));
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"o_orderpriority\torder_count\nA\t1\n");
    let query = "SELECT o_orderpriority, COUNT(*) AS n FROM orders WHERE o_orderdate < DATE '1993-01-01' AND EXISTS (SELECT * FROM lineitem WHERE l_orderkey = o_orderkey AND l_commitdate < l_receiptdate) GROUP BY o_orderpriority ORDER BY o_orderpriority";
    let empty = sql(query);
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"o_orderpriority\tn\n");
    let no_keys = sql(
        "SELECT o_orderpriority, COUNT(*) AS n FROM orders WHERE o_orderdate < DATE '2000-01-01' AND EXISTS (SELECT * FROM lineitem WHERE o_orderkey = l_orderkey AND l_commitdate < DATE '1970-01-01') GROUP BY o_orderpriority ORDER BY o_orderpriority",
    );
    assert!(no_keys.status.success(), "{no_keys:?}");
    assert_eq!(no_keys.stdout, b"o_orderpriority\tn\n");
    let exists_first = sql(
        "SELECT COUNT(*) FROM orders WHERE EXISTS (SELECT * FROM lineitem WHERE l_orderkey = o_orderkey AND l_commitdate < l_receiptdate) AND o_orderdate < DATE '2000-01-01'",
    );
    assert!(exists_first.status.success(), "{exists_first:?}");
    assert_eq!(exists_first.stdout, b"count\n2\n");
    let exists_only = sql(
        "SELECT COUNT(*) FROM orders WHERE EXISTS (SELECT * FROM lineitem WHERE l_orderkey = o_orderkey AND l_commitdate < l_receiptdate)",
    );
    assert!(exists_only.status.success(), "{exists_only:?}");
    assert_eq!(exists_only.stdout, b"count\n2\n");
    assert!(
        sql("CREATE TABLE empty_lines (l_key Nullable(UInt64))")
            .status
            .success()
    );
    let empty_table = sql(
        "SELECT COUNT(*) FROM orders WHERE EXISTS (SELECT * FROM empty_lines WHERE l_key = o_orderkey)",
    );
    assert!(empty_table.status.success(), "{empty_table:?}");
    assert_eq!(empty_table.stdout, b"count\n0\n");
    let statement = "CREATE TABLE wrong_type (l_key Int64)";
    assert!(sql(statement).status.success());
    for query in [
        "SELECT count(*) FROM orders WHERE o_orderkey = 1 AND EXISTS (SELECT * FROM wrong_type WHERE l_key = o_orderkey)",
        "SELECT count(*) FROM orders WHERE o_orderkey = 1 OR o_orderkey = 2 AND EXISTS (SELECT * FROM lineitem WHERE l_orderkey = o_orderkey)",
        "SELECT count(*) FROM orders WHERE o_orderkey = 1 AND NOT EXISTS (SELECT * FROM lineitem WHERE l_orderkey = o_orderkey)",
        "SELECT count(*) FROM orders WHERE o_orderkey = 1 AND EXISTS (SELECT * FROM lineitem WHERE l_orderkey = o_orderkey LIMIT 0)",
    ] {
        let result = sql(query);
        assert!(!result.status.success(), "{query}: {result:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
