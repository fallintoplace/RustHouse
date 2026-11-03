use std::fs;
use std::process::Command;

#[test]
fn q12_groups_filtered_join_rows_with_null_safe_case_counts() {
    let db = std::env::temp_dir().join(format!("rusthouse-q12-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE orders (o_orderkey UInt64, o_orderpriority Nullable(String))",
        "CREATE TABLE lineitem (l_orderkey UInt64, l_shipmode Nullable(String), l_shipdate Date, l_commitdate Date, l_receiptdate Date)",
        "INSERT INTO orders VALUES (1, '1-URGENT'), (2, '5-LOW'), (3, NULL), (4, '2-HIGH'), (5, '3-MEDIUM')",
        "INSERT INTO lineitem VALUES (1, 'MAIL', '1994-01-01', '1994-01-02', '1994-01-03'), (1, 'MAIL', '1994-01-01', '1994-01-02', '1994-01-03'), (2, 'MAIL', '1994-01-01', '1994-01-02', '1994-01-03'), (3, 'SHIP', '1994-01-01', '1994-01-02', '1994-01-03'), (4, 'SHIP', '1994-01-01', '1994-01-03', '1994-01-03'), (5, 'AIR', '1994-01-01', '1994-01-02', '1994-01-03'), (5, NULL, '1994-01-01', '1994-01-02', '1994-01-03')",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let output = sql(include_str!("../../../bench/tpch/queries/q12.sql"));
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        output.stdout,
        b"l_shipmode\thigh_line_count\tlow_line_count\nMAIL\t2\t1\nSHIP\t0\t0\n"
    );
    let query = "SELECT l_shipmode, SUM(CASE WHEN o_orderpriority = '1-URGENT' THEN 1 ELSE 0 END) AS n FROM orders o JOIN lineitem l ON o.o_orderkey = l.l_orderkey GROUP BY l_shipmode ORDER BY l_shipmode NULLS LAST";
    let output = sql(query);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        output.stdout,
        b"l_shipmode\tn\nAIR\t0\nMAIL\t2\nSHIP\t0\n\\N\t0\n"
    );
    let any = sql(
        "SELECT l_shipmode, SUM(CASE WHEN o_orderpriority = '1-URGENT' THEN 1 ELSE 0 END) AS n FROM orders o ANY INNER JOIN lineitem l ON o.o_orderkey = l.l_orderkey GROUP BY l_shipmode ORDER BY l_shipmode",
    );
    assert!(any.status.success(), "{any:?}");
    assert_eq!(any.stdout, b"l_shipmode\tn\nAIR\t0\nMAIL\t1\nSHIP\t0\n");
    let empty = sql(
        "SELECT l_shipmode, SUM(CASE WHEN o_orderpriority = '1-URGENT' THEN 1 ELSE 0 END) AS n FROM orders o JOIN lineitem l ON o.o_orderkey = l.l_orderkey WHERE l_orderkey = 999 GROUP BY l_shipmode ORDER BY l_shipmode",
    );
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(empty.stdout, b"l_shipmode\tn\n");
    let page = sql(
        "SELECT l_shipmode, SUM(CASE WHEN o_orderpriority = '1-URGENT' THEN 1 ELSE 0 END) AS n FROM orders o JOIN lineitem l ON o.o_orderkey = l.l_orderkey GROUP BY l_shipmode ORDER BY l_shipmode DESC NULLS LAST LIMIT 2 OFFSET 1",
    );
    assert!(page.status.success(), "{page:?}");
    assert_eq!(page.stdout, b"l_shipmode\tn\nMAIL\t2\nAIR\t0\n");
    let zero = sql(
        "SELECT l_shipmode, SUM(CASE WHEN o_orderpriority = '1-URGENT' THEN 1 ELSE 0 END) AS n FROM orders o JOIN lineitem l ON o.o_orderkey = l.l_orderkey GROUP BY l_shipmode ORDER BY l_shipmode LIMIT 0",
    );
    assert!(zero.status.success(), "{zero:?}");
    assert_eq!(zero.stdout, b"l_shipmode\tn\n");
    for query in [
        "SELECT l_shipmode, SUM(CASE WHEN o_orderpriority = '1-URGENT' THEN 1 ELSE 0 END) AS n FROM orders o JOIN lineitem l ON o.o_orderkey = l.l_orderkey GROUP BY l_shipmode ORDER BY count",
        "SELECT l_shipmode, SUM(CASE WHEN o_orderpriority = '1-URGENT' THEN 1 ELSE 2 END) AS n FROM orders o JOIN lineitem l ON o.o_orderkey = l.l_orderkey GROUP BY l_shipmode",
        "SELECT l_shipmode, SUM(CASE WHEN missing = 'x' THEN 1 ELSE 0 END) AS n FROM orders o JOIN lineitem l ON o.o_orderkey = l.l_orderkey GROUP BY l_shipmode LIMIT 0",
        "SELECT l_shipmode, SUM(CASE WHEN o_orderpriority = '1-URGENT' THEN 1 ELSE 0 END) AS n FROM orders o LEFT JOIN lineitem l ON o.o_orderkey = l.l_orderkey GROUP BY l_shipmode",
    ] {
        let output = sql(query);
        assert!(!output.status.success(), "{query}: {output:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
