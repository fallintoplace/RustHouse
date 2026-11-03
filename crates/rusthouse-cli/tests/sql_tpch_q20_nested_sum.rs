use std::fs;
use std::process::Command;

#[test]
fn q20_filters_suppliers_by_nested_membership_and_pair_sum() {
    let db = std::env::temp_dir().join(format!("rusthouse-q20-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE supplier (s_suppkey UInt64, s_nationkey UInt64, s_name String, s_address String)",
        "CREATE TABLE nation (n_nationkey UInt64, n_name String)",
        "CREATE TABLE part (p_partkey UInt64, p_name String)",
        "CREATE TABLE partsupp (ps_suppkey UInt64, ps_partkey UInt64, ps_availqty Int64)",
        "CREATE TABLE lineitem (l_partkey UInt64, l_suppkey UInt64, l_quantity Nullable(Float64), l_shipdate Date)",
        "INSERT INTO nation VALUES (1, 'CANADA'), (2, 'USA')",
        "INSERT INTO supplier VALUES (1, 1, 'A', 'a'), (2, 1, 'B', 'b'), (3, 2, 'C', 'c'), (4, 1, 'D', 'd'), (5, 1, 'E', 'e'), (6, 1, 'F', 'f')",
        "INSERT INTO part VALUES (1, 'forest green'), (1, 'forest tree'), (2, 'red'), (3, 'forest oak')",
        "INSERT INTO partsupp VALUES (1, 1, 11), (2, 1, 10), (3, 1, 11), (4, 3, 99), (5, 3, 99), (6, 3, 3), (2, 2, 99)",
        "INSERT INTO lineitem VALUES (1, 1, 20, '1994-01-01'), (1, 1, 999, '1995-01-01'), (1, 2, 20, '1994-12-31'), (1, 3, 2, '1994-06-01'), (3, 5, NULL, '1994-06-01'), (3, 6, 4, '1994-06-01'), (2, 2, 1, '1994-06-01')",
    ] {
        let out = sql(statement);
        assert!(
            out.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let query = include_str!("../../../bench/tpch/queries/q20.sql");
    let result = sql(query);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"s_name\ts_address\nA\ta\nF\tf\n");
    let qualified = query.replace("s_suppkey IN (", "supplier.s_suppkey IN (");
    let result = sql(&qualified);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"s_name\ts_address\nA\ta\nF\tf\n");
    let limited = query.trim_end().trim_end_matches(';').to_owned() + " LIMIT 1 OFFSET 1";
    let result = sql(&limited);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"s_name\ts_address\nF\tf\n");
    let zero = query.trim_end().trim_end_matches(';').to_owned() + " LIMIT 0";
    let result = sql(&zero);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"s_name\ts_address\n");
    for invalid in [
        query.replace("l_partkey = ps_partkey", "l_partkey = part.p_partkey"),
        query.replace("p_name LIKE", "missing LIKE"),
        query.replace("sum(l_quantity)", "sum(missing)"),
        query.replace(
            "ORDER BY\n            s_name",
            "ORDER BY\n            missing",
        ),
        query.replace(
            "p_name LIKE 'forest%'",
            "p_name LIKE 'forest%' OR p_name LIKE 'red%'",
        ),
    ] {
        assert!(!sql(&invalid).status.success(), "{invalid}");
    }
    fs::remove_dir_all(db).unwrap();
}
