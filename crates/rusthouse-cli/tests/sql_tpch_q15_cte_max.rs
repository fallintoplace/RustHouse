use std::fs;
use std::process::Command;

#[test]
fn q15_joins_all_maximum_revenue_ties() {
    let db = std::env::temp_dir().join(format!("rusthouse-q15-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE supplier (s_suppkey UInt64, s_name String, s_address String, s_phone String)",
        "CREATE TABLE bad_supplier (s_suppkey UInt32, s_name String, s_address String, s_phone String)",
        "CREATE TABLE lineitem (l_suppkey Nullable(UInt64), l_extendedprice Nullable(Float64), l_discount Float64, l_shipdate Date)",
        "CREATE TABLE collision_lineitem (l_suppkey Nullable(UInt64), supplier_no UInt64, l_extendedprice Float64, l_discount Float64, l_shipdate Date)",
        "INSERT INTO supplier VALUES (1, 'A', 'one', '11'), (2, 'B', 'two', '22'), (3, 'C', 'three', '33'), (4, 'D', 'four', '44')",
        "INSERT INTO lineitem VALUES (1, 100, 0.1, '1996-01-01'), (1, 10, 0, '1996-02-01'), (2, 100, 0, '1996-03-01'), (3, 2, 0, '1996-01-02'), (4, NULL, 0, '1996-01-02'), (2, 1000, 0, '1995-01-01')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = include_str!("../../../bench/tpch/queries/q15.sql");
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"s_suppkey\ts_name\ts_address\ts_phone\ttotal_revenue\n1\tA\tone\t11\t100\n2\tB\ttwo\t22\t100\n");
    let limited = query.trim_end().trim_end_matches(';').to_owned() + " LIMIT 1 OFFSET 1";
    let result = sql(&limited);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"s_suppkey\ts_name\ts_address\ts_phone\ttotal_revenue\n2\tB\ttwo\t22\t100\n"
    );
    let zero = query.trim_end().trim_end_matches(';').to_owned() + " LIMIT 0";
    let result = sql(&zero);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"s_suppkey\ts_name\ts_address\ts_phone\ttotal_revenue\n"
    );
    assert!(
        !sql(&zero.replace("l_shipdate >=", "missing >="))
            .status
            .success()
    );
    let json = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(&db)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(json.status.success(), "{json:?}");
    assert!(String::from_utf8_lossy(&json.stdout).contains("\"s_name\":\"A\""));
    for invalid in [
        query.replace("FROM\n        lineitem", "FROM\n        collision_lineitem"),
        query.replace("FROM\n    supplier,", "FROM\n    bad_supplier,"),
        query.replace("max(total_revenue)", "min(total_revenue)"),
        query.replace("s_suppkey = supplier_no", "s_suppkey > supplier_no"),
        query.replace(
            "GROUP BY\n        supplier_no",
            "GROUP BY\n        l_discount",
        ),
    ] {
        assert!(!sql(&invalid).status.success(), "{invalid}");
    }
    assert!(
        sql("INSERT INTO lineitem VALUES (99, 200, 0, '1996-01-01')")
            .status
            .success()
    );
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"s_suppkey\ts_name\ts_address\ts_phone\ttotal_revenue\n"
    );
    assert!(sql("TRUNCATE TABLE lineitem").status.success());
    assert!(sql("INSERT INTO lineitem VALUES (1, 1000000000000000100, 0, '1996-01-01'), (2, 1000000000000000100, 0, '1996-01-01'), (3, 1000000000000000000, 0, '1996-01-01')").status.success());
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    let text = String::from_utf8(result.stdout).unwrap();
    assert_eq!(
        text.lines()
            .skip(1)
            .map(|line| line.split('\t').next().unwrap())
            .collect::<Vec<_>>(),
        vec!["1", "2"]
    );
    assert!(sql("TRUNCATE TABLE lineitem").status.success());
    assert!(
        sql("INSERT INTO lineitem VALUES (NULL, 200, 0, '1996-01-01'), (1, 100, 0, '1996-01-01')")
            .status
            .success()
    );
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"s_suppkey\ts_name\ts_address\ts_phone\ttotal_revenue\n"
    );
    assert!(sql("TRUNCATE TABLE lineitem").status.success());
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"s_suppkey\ts_name\ts_address\ts_phone\ttotal_revenue\n"
    );
    fs::remove_dir_all(db).unwrap();
}
