use std::fs;
use std::process::Command;

#[test]
fn q21_applies_correlated_exists_before_grouping() {
    let db = std::env::temp_dir().join(format!("rusthouse-q21-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE nation (n_nationkey UInt64, n_name String)",
        "CREATE TABLE supplier (s_suppkey UInt64, s_nationkey UInt64, s_name String)",
        "CREATE TABLE orders (o_orderkey UInt64, o_orderstatus String)",
        "CREATE TABLE lineitem (l_orderkey Nullable(UInt64), l_suppkey Nullable(UInt64), l_receiptdate Nullable(Date), l_commitdate Nullable(Date))",
        "INSERT INTO nation VALUES (1, 'SAUDI ARABIA'), (2, 'FRANCE')",
        "INSERT INTO supplier VALUES (1, 1, 'A'), (2, 2, 'B')",
        "INSERT INTO orders VALUES (11, 'F'), (12, 'F'), (13, 'F'), (15, 'F')",
        "INSERT INTO lineitem VALUES (11, 1, '1995-01-03', '1995-01-02'), (11, 2, '1995-01-01', '1995-01-02'), (11, NULL, '1995-01-03', '1995-01-02'), (12, 1, '1995-01-03', '1995-01-02'), (12, 2, '1995-01-03', '1995-01-02'), (13, 1, '1995-01-03', '1995-01-02'), (13, 1, '1995-01-03', '1995-01-02'), (15, 1, '1995-01-03', '1995-01-02'), (15, 2, NULL, '1995-01-02'), (NULL, 2, '1995-01-03', '1995-01-02')",
    ] {
        let output = sql(statement);
        assert!(output.status.success(), "{statement}: {output:?}");
    }
    let query = include_str!("../../../bench/tpch/queries/q21.sql");
    let output = sql(query);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"s_name\tnumwait\nA\t2\n");
    let explicit = query.replace("supplier,\n    lineitem l1,\n    orders,\n    nation", "supplier JOIN lineitem l1 ON supplier.s_suppkey = l1.l_suppkey JOIN orders ON orders.o_orderkey = l1.l_orderkey JOIN nation ON supplier.s_nationkey = nation.n_nationkey");
    let output = sql(&explicit);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"s_name\tnumwait\nA\t2\n");
    let paged = query.replace("LIMIT 100", "LIMIT 1 OFFSET 1");
    let output = sql(&paged);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"s_name\tnumwait\n");
    let empty = query.replace("'SAUDI ARABIA'", "'MISSING'");
    let output = sql(&empty);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"s_name\tnumwait\n");
    let incompatible = query.replace(
        "l2.l_orderkey = l1.l_orderkey",
        "l2.l_orderkey = supplier.s_name",
    );
    assert!(!sql(&incompatible).status.success());
    let unknown = query.replace(
        "l3.l_receiptdate > l3.l_commitdate",
        "l3.missing > l3.l_commitdate",
    );
    assert!(!sql(&unknown).status.success());
    let invalid_or = query.replace("AND EXISTS (", "OR EXISTS (");
    assert!(!sql(&invalid_or).status.success());
    let json = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(&db)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(json.status.success(), "{json:?}");
    assert_eq!(json.stdout, b"[{\"s_name\":\"A\",\"numwait\":\"2\"}]\n");
    let case = "SELECT n_name, SUM(CASE WHEN s_suppkey = 1 THEN 1 ELSE 0 END) AS n FROM supplier, nation WHERE s_nationkey = n_nationkey AND EXISTS (SELECT * FROM lineitem l2 WHERE l2.l_suppkey = supplier.s_suppkey) GROUP BY n_name";
    assert!(!sql(case).status.success());
    fs::remove_dir_all(db).unwrap();
}
