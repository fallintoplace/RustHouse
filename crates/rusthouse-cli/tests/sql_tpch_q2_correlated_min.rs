use std::fs;
use std::process::Command;

#[test]
fn q2_filters_to_minimum_supply_cost_in_region() {
    let db = std::env::temp_dir().join(format!("rusthouse-q2-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE part (p_partkey UInt64, p_size UInt64, p_type String, p_mfgr String)",
        "CREATE TABLE supplier (s_suppkey UInt64, s_nationkey UInt64, s_acctbal Float64, s_name String, s_address String, s_phone String, s_comment String)",
        "CREATE TABLE partsupp (ps_partkey UInt64, ps_suppkey UInt64, ps_supplycost Nullable(Float64))",
        "CREATE TABLE nation (n_nationkey UInt64, n_regionkey UInt64, n_name String)",
        "CREATE TABLE region (r_regionkey UInt64, r_name String)",
        "INSERT INTO part VALUES (1, 15, 'SMALL BRASS', 'M1'), (2, 15, 'BIG BRASS', 'M2'), (3, 15, 'STEEL', 'M3'), (4, 15, 'BRASS', 'M4')",
        "INSERT INTO region VALUES (1, 'EUROPE'), (2, 'AMERICA')",
        "INSERT INTO nation VALUES (1, 1, 'FRANCE'), (2, 1, 'GERMANY'), (3, 2, 'USA')",
        "INSERT INTO supplier VALUES (11, 1, 50, 'A', 'a', '1', 'aa'), (22, 2, 80, 'B', 'b', '2', 'bb'), (33, 3, 100, 'C', 'c', '3', 'cc')",
        "INSERT INTO partsupp VALUES (1, 11, 10), (1, 22, 10), (1, 33, 1), (2, 11, 30), (2, 22, 20), (3, 22, 1), (4, 11, NULL), (4, 22, NULL)",
    ] {
        let out = sql(statement);
        assert!(
            out.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let query = include_str!("../../../bench/tpch/queries/q02.sql");
    let result = sql(query);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"s_acctbal\ts_name\tn_name\tp_partkey\tp_mfgr\ts_address\ts_phone\ts_comment\n80\tB\tGERMANY\t1\tM1\tb\t2\tbb\n80\tB\tGERMANY\t2\tM2\tb\t2\tbb\n50\tA\tFRANCE\t1\tM1\ta\t1\taa\n");
    let zero = query.replace("LIMIT 100;", "LIMIT 0;");
    let result = sql(&zero);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        result.stdout,
        b"s_acctbal\ts_name\tn_name\tp_partkey\tp_mfgr\ts_address\ts_phone\ts_comment\n"
    );
    for invalid in [
        query.replace("min(ps_supplycost)", "min(missing)"),
        query.replace(
            "p_partkey = ps_partkey\n            AND",
            "p_missing = ps_partkey\n            AND",
        ),
        query.replace("ps_supplycost = (", "s_name = ("),
        query.replace(
            "r_name = 'EUROPE')",
            "r_name = 'EUROPE' OR r_name = 'AMERICA')",
        ),
    ] {
        assert!(!sql(&invalid).status.success(), "{invalid}");
    }
    let shadowed = "SELECT p.p_partkey FROM part p JOIN partsupp o ON p.p_partkey = o.ps_partkey WHERE o.ps_supplycost = (SELECT MIN(i.ps_supplycost) FROM partsupp i, supplier p WHERE i.ps_partkey = p.p_partkey AND i.ps_suppkey = p.s_suppkey)";
    assert!(!sql(shadowed).status.success(), "{shadowed}");
    fs::remove_dir_all(db).unwrap();
}
