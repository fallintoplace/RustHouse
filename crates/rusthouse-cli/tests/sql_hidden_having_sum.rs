use std::fs;
use std::process::Command;

#[test]
fn having_can_filter_a_sum_not_in_the_output() {
    let db = std::env::temp_dir().join(format!("rusthouse-hidden-having-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE lineitem (l_orderkey Nullable(UInt64), l_quantity Nullable(Float64))",
        "INSERT INTO lineitem VALUES (1, 100), (1, 201), (2, 300), (3, NULL), (NULL, 400)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT l_orderkey FROM lineitem GROUP BY l_orderkey HAVING SUM(l_quantity) > 300 ORDER BY l_orderkey NULLS LAST",
            "l_orderkey\n1\n\\N\n",
        ),
        (
            "SELECT l_orderkey FROM lineitem WHERE l_quantity < 300 GROUP BY l_orderkey HAVING SUM(l_quantity) > 300",
            "l_orderkey\n1\n",
        ),
        (
            "SELECT l_orderkey AS k FROM lineitem GROUP BY l_orderkey HAVING SUM(l_quantity) > 300 ORDER BY k NULLS LAST",
            "k\n1\n\\N\n",
        ),
        (
            "SELECT l_orderkey AS k FROM lineitem GROUP BY l_orderkey HAVING SUM(l_quantity) > 300 ORDER BY ALL NULLS LAST",
            "k\n1\n\\N\n",
        ),
        (
            "SELECT l_orderkey AS k FROM lineitem GROUP BY l_orderkey HAVING SUM(l_quantity) > 300 ORDER BY ALL NULLS LAST LIMIT 0",
            "k\n",
        ),
        (
            "SELECT l_orderkey FROM lineitem GROUP BY l_orderkey HAVING SUM(l_quantity) > 300 LIMIT 0",
            "l_orderkey\n",
        ),
        (
            "SELECT l_orderkey FROM lineitem GROUP BY l_orderkey HAVING SUM(l_quantity) > 900",
            "l_orderkey\n",
        ),
    ] {
        let result = sql(query);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(result.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT l_orderkey FROM lineitem GROUP BY l_orderkey HAVING SUM(missing) > 3 LIMIT 0",
        "SELECT l_orderkey FROM lineitem GROUP BY l_orderkey HAVING SUM(l_quantity) > 3 ORDER BY 2 LIMIT 0",
        "SELECT l_orderkey FROM lineitem GROUP BY l_quantity HAVING SUM(l_quantity) > 3",
        "SELECT l_orderkey FROM lineitem GROUP BY l_orderkey HAVING SUM(l_quantity) > -1",
        "SELECT number FROM numbers(4) GROUP BY number HAVING SUM(number) > 1",
    ] {
        let result = sql(query);
        assert!(!result.status.success(), "{query}: {result:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
