use std::fs;
use std::process::Command;

#[test]
fn q13_groups_left_join_counts_including_zeroes() {
    let db = std::env::temp_dir().join(format!("rusthouse-q13-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE customer (c_custkey UInt64)",
        "CREATE TABLE orders (o_orderkey Nullable(UInt64), o_custkey Nullable(UInt64), o_comment Nullable(String))",
        "CREATE TABLE ambiguous_orders (o_orderkey UInt64, o_custkey UInt64, c_custkey UInt64, o_comment String)",
        "INSERT INTO customer VALUES (1), (2), (3), (4), (5)",
        "INSERT INTO orders VALUES (11, 1, 'regular'), (12, 1, 'special urgent requests'), (21, 2, 'regular'), (22, 2, 'regular'), (31, 3, 'special requests'), (51, 5, NULL), (NULL, 5, 'regular'), (99, NULL, 'regular')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = include_str!("../../../bench/tpch/queries/q13.sql");
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"c_count\tcustdist\n0\t3\n2\t1\n1\t1\n");
    let limited = query.trim_end().trim_end_matches(';').to_owned() + " LIMIT 1 OFFSET 1";
    let result = sql(&limited);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"c_count\tcustdist\n2\t1\n");
    let zero = query.trim_end().trim_end_matches(';').to_owned() + " LIMIT 0";
    let result = sql(&zero);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"c_count\tcustdist\n");
    let mut json = Command::new(env!("CARGO_BIN_EXE_rusthouse"));
    let result = json
        .args(["sql", query, "--database"])
        .arg(&db)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(String::from_utf8_lossy(&result.stdout).contains("\"c_count\":\"0\""));
    for invalid in [
        query.replace("LEFT OUTER JOIN orders", "LEFT OUTER JOIN ambiguous_orders"),
        query.replace("c_custkey = o_custkey", "c_custkey = missing"),
        query.replace("count(o_orderkey)", "count(missing)"),
        query.replace("o_comment NOT LIKE", "o_orderkey NOT LIKE"),
        query.replace("GROUP BY\n    c_count", "GROUP BY\n    c_custkey"),
        query
            .replace("c_count)", "\"#aggregate0\")")
            .replace("c_count,", "\"#aggregate0\",")
            .replace("    c_count\n", "    \"#aggregate0\"\n")
            .replace("    c_count DESC", "    \"#aggregate0\" DESC"),
    ] {
        assert!(!sql(&invalid).status.success(), "{invalid}");
    }
    assert!(sql("TRUNCATE TABLE customer").status.success());
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"c_count\tcustdist\n");
    fs::remove_dir_all(db).unwrap();
}
