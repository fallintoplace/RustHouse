use std::fs;
use std::process::Command;

#[test]
fn q11_compares_grouped_sum_to_scalar_join_subquery() {
    let db = std::env::temp_dir().join(format!("rusthouse-q11-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE nation (n_nationkey UInt64, n_name String)",
        "CREATE TABLE supplier (s_suppkey UInt64, s_nationkey UInt64)",
        "CREATE TABLE partsupp (ps_partkey UInt64, ps_suppkey UInt64, ps_supplycost Float64, ps_availqty UInt64)",
        "INSERT INTO nation VALUES (1, 'GERMANY'), (2, 'FRANCE')",
        "INSERT INTO supplier VALUES (1, 1), (2, 2)",
        "INSERT INTO partsupp VALUES (10, 1, 2.5, 4), (10, 1, 1, 2), (11, 1, 3, 2), (12, 2, 100, 1)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = include_str!("../../../bench/tpch/queries/q11.sql");
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"ps_partkey\tvalue\n10\t12\n11\t6\n");
    let empty_inner = query.rsplit_once("n_name = 'GERMANY'").unwrap();
    let empty = format!(
        "{}n_name = 'FRANCE-MISSING'{}",
        empty_inner.0, empty_inner.1
    );
    let result = sql(&empty);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"ps_partkey\tvalue\n");
    for invalid in [
        query.replace("sum(ps_supplycost * ps_availqty) * 0.0001000000", "sum(ps_supplycost * ps_availqty) * 0.0001000000 + missing"),
        query.replace("ORDER BY\n    value DESC", "ORDER BY value DESC LIMIT (SELECT SUM(ps_supplycost * 1) FROM partsupp p JOIN supplier s ON p.ps_suppkey = s.s_suppkey)"),
        query.replace("AND n_name = 'GERMANY')", "AND n_name = 'GERMANY' GROUP BY ps_partkey)"),
    ] {
        assert!(!sql(&invalid).status.success(), "{invalid}");
    }
    for statement in [
        "CREATE TABLE outer_a (id UInt64, v UInt64)",
        "CREATE TABLE outer_b (id UInt64)",
        "CREATE TABLE inner_a (id UInt64, v Float64)",
        "CREATE TABLE inner_b (id UInt64)",
        "INSERT INTO outer_a VALUES (1, 1000000000000000100)",
        "INSERT INTO outer_b VALUES (1)",
        "INSERT INTO inner_a VALUES (1, 1000000000000000100)",
        "INSERT INTO inner_b VALUES (1)",
    ] {
        assert!(sql(statement).status.success(), "{statement}");
    }
    let precision = "SELECT outer_a.id, SUM(outer_a.v) AS total FROM outer_a JOIN outer_b ON outer_a.id = outer_b.id GROUP BY outer_a.id HAVING total < (SELECT SUM(v * 1) FROM inner_a JOIN inner_b ON inner_a.id = inner_b.id)";
    let result = sql(precision);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"outer_a.id\ttotal\n1\t1000000000000000100\n"
    );
    for statement in [
        "CREATE TABLE x (x_id UInt64, amount UInt64)",
        "CREATE TABLE y (y_id UInt64)",
        "INSERT INTO x VALUES (1, 2), (2, 4)",
        "INSERT INTO y VALUES (1), (2)",
    ] {
        assert!(sql(statement).status.success(), "{statement}");
    }
    let nested_comma = "SELECT x.x_id, SUM(x.amount) AS total FROM x JOIN y ON x.x_id = y.y_id GROUP BY x.x_id HAVING total > (SELECT SUM(amount * 0.5) FROM x, y WHERE x_id = y_id) ORDER BY x.x_id";
    let result = sql(nested_comma);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"x.x_id\ttotal\n2\t4\n");
    fs::remove_dir_all(db).unwrap();
}
