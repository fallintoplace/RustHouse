use std::fs;
use std::process::Command;

#[test]
fn q22_groups_above_average_customers_without_orders() {
    let db = std::env::temp_dir().join(format!("rusthouse-q22-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE customer (c_custkey Nullable(UInt64), c_phone Nullable(String), c_acctbal Nullable(Float64))",
        "CREATE TABLE orders (o_custkey Nullable(UInt64))",
        "INSERT INTO customer VALUES (1, '13-a', 100), (2, '31-a', 200), (3, '13-b', 300), (4, '99-a', 10000), (5, '23-a', -5), (6, '13-c', NULL), (7, NULL, 100), (8, '17-a', 200), (NULL, '31-x', 500)",
        "INSERT INTO orders VALUES (2), (NULL)",
    ] {
        let out = sql(statement);
        assert!(
            out.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let query = include_str!("../../../bench/tpch/queries/q22.sql");
    let result = sql(query);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        result.stdout,
        b"cntrycode\tnumcust\ttotacctbal\n13\t1\t300\n31\t1\t500\n"
    );
    let limited = query.trim_end().trim_end_matches(';').to_owned() + " LIMIT 1 OFFSET 1";
    assert_eq!(
        sql(&limited).stdout,
        b"cntrycode\tnumcust\ttotacctbal\n31\t1\t500\n"
    );
    let zero = query.trim_end().trim_end_matches(';').to_owned() + " LIMIT 0";
    assert_eq!(sql(&zero).stdout, b"cntrycode\tnumcust\ttotacctbal\n");
    for invalid in [
        query
            .replace("AS cntrycode", "AS c_acctbal")
            .replace("cntrycode", "c_acctbal"),
        query.replace("o_custkey = c_custkey", "o_custkey = missing"),
        query.replace("c_acctbal > 0.00", "c_acctbal > 0.00 OR c_acctbal < 0.00"),
    ] {
        assert!(!sql(&invalid).status.success(), "{invalid}");
    }
    assert!(sql("TRUNCATE TABLE customer").status.success());
    assert!(
        sql("INSERT INTO customer VALUES (1, '13-a', NULL), (2, '13-b', -1)")
            .status
            .success()
    );
    assert_eq!(sql(query).stdout, b"cntrycode\tnumcust\ttotacctbal\n");
    assert!(sql("TRUNCATE TABLE customer").status.success());
    assert!(sql("INSERT INTO customer VALUES (10, 'é😀-a', 100), (11, 'é😀-b', 300), (12, '31-a', 500), (13, 'é😀-c', 700)").status.success());
    let unicode = query.replace("'13'", "'é😀'");
    let result = sql(&unicode);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "cntrycode\tnumcust\ttotacctbal\n31\t1\t500\né😀\t1\t700\n"
    );
    fs::remove_dir_all(db).unwrap();
}
