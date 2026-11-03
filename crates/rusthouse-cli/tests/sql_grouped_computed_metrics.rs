use std::fs;
use std::process::Command;

#[test]
fn grouped_arithmetic_handles_null_and_multiple_keys() {
    let db =
        std::env::temp_dir().join(format!("rusthouse-grouped-computed-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE items (category UInt64, kind UInt64, price Float64, discount Nullable(Float64))",
        "INSERT INTO items VALUES (1, 1, 10.0, 0.1), (1, 1, 20.0, 0.2), (1, 2, 50.0, NULL), (2, 1, 4.0, 0.5)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = "SELECT category, kind, SUM(price * (1 - discount)) AS revenue, COUNT(*) AS n FROM items GROUP BY category, kind ORDER BY category, kind";
    let output = sql(query);
    assert!(output.status.success(), "{query}: {output:?}");
    assert_eq!(
        output.stdout,
        b"category\tkind\trevenue\tn\n1\t1\t25\t2\n1\t2\tNULL\t1\n2\t1\t2\t1\n"
    );
    let unaliased = sql(
        "SELECT category, SUM(price * (1 - discount)) FROM items GROUP BY category ORDER BY category",
    );
    assert!(unaliased.status.success(), "{unaliased:?}");
    assert_eq!(
        unaliased.stdout,
        b"category\tsum(price * (1 - discount))\n1\t25\n2\t2\n"
    );
    let query = "SELECT category, SUM(price * (1 - discount)) AS revenue, COUNT(*) AS n FROM items GROUP BY category ORDER BY category";
    let output = sql(query);
    assert!(output.status.success(), "{query}: {output:?}");
    assert_eq!(output.stdout, b"category\trevenue\tn\n1\t25\t3\n2\t2\t1\n");
    let mixed = sql(
        "SELECT category, kind, COUNT(DISTINCT price) AS unique_prices, SUM(price * (1 - discount)) AS revenue FROM items GROUP BY category, kind ORDER BY category, kind",
    );
    assert!(mixed.status.success(), "{mixed:?}");
    assert_eq!(
        mixed.stdout,
        b"category\tkind\tunique_prices\trevenue\n1\t1\t2\t25\n1\t2\t1\tNULL\n2\t1\t1\t2\n"
    );
    let bad = sql("SELECT category, SUM(price / 0) AS total FROM items GROUP BY category");
    assert!(!bad.status.success(), "{bad:?}");
    assert!(String::from_utf8_lossy(&bad.stderr).contains("division by zero"));
    let bad = sql("SELECT category, SUM(missing + 1) AS total FROM items GROUP BY category");
    assert!(!bad.status.success(), "{bad:?}");
    let generated = sql("SELECT number, SUM(number + 1) AS total FROM numbers(3) GROUP BY number");
    assert!(!generated.status.success(), "{generated:?}");
    assert!(String::from_utf8_lossy(&generated.stderr).contains("require a stored table"));
    for statement in [
        "CREATE TABLE lineitem (l_returnflag String, l_linestatus String, l_quantity Float64, l_extendedprice Float64, l_discount Float64, l_tax Float64, l_shipdate Date)",
        "INSERT INTO lineitem VALUES ('A','F',2.0,10.0,0.1,0.0,'1998-01-01'), ('A','F',4.0,20.0,0.2,0.0,'1998-01-02'), ('B','O',1.0,10.0,0.0,0.0,'1999-01-01')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = include_str!("../../../bench/tpch/queries/q01.sql");
    let output = sql(query);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"l_returnflag\tl_linestatus\tsum_qty\tsum_base_price\tsum_disc_price\tsum_charge\tavg_qty\tavg_price\tavg_disc\tcount_order\nA\tF\t6\t30\t25\t25\t3\t15\t0.15000000000000002\t2\n");
    fs::remove_dir_all(db).unwrap();
}
