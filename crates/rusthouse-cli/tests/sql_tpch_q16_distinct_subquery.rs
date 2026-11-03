use std::fs;
use std::process::Command;

#[test]
fn q16_counts_suppliers_after_not_in_subquery() {
    let db = std::env::temp_dir().join(format!("rusthouse-tpch-q16-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE part (p_partkey UInt64, p_brand String, p_type String, p_size Int64)",
        "CREATE TABLE partsupp (ps_partkey UInt64, ps_suppkey Nullable(UInt64))",
        "CREATE TABLE supplier (s_suppkey Nullable(UInt64), s_comment String)",
        "CREATE TABLE empty_supplier (s_suppkey Nullable(UInt64), s_comment String)",
        "CREATE TABLE null_supplier (s_suppkey Nullable(UInt64), s_comment String)",
        "CREATE TABLE many_supplier (s_suppkey Nullable(UInt64), s_comment String)",
        "INSERT INTO part VALUES (1, 'Brand#11', 'STANDARD', 3), (2, 'Brand#12', 'STANDARD', 3), (3, 'Brand#13', 'STANDARD', 3), (4, 'Brand#45', 'STANDARD', 3)",
        "INSERT INTO partsupp VALUES (1, 10), (1, 11), (1, 12), (1, 11), (2, 10), (2, 11), (3, NULL), (4, 11)",
        "INSERT INTO supplier VALUES (10, 'Customer many Complaints'), (11, 'ordinary'), (12, 'ordinary')",
        "INSERT INTO empty_supplier VALUES (99, 'ordinary')",
        "INSERT INTO null_supplier VALUES (NULL, 'Customer Complaints'), (10, 'Customer Complaints')",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let values = (20..40)
        .map(|key| format!("({key}, 'Customer Complaints')"))
        .collect::<Vec<_>>()
        .join(", ");
    let result = sql(&format!("INSERT INTO many_supplier VALUES {values}"));
    assert!(result.status.success(), "{result:?}");
    let query = include_str!("../../../bench/tpch/queries/q16.sql");
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"p_brand\tp_type\tp_size\tsupplier_cnt\nBrand#11\tSTANDARD\t3\t2\nBrand#12\tSTANDARD\t3\t1\n");
    let empty = query.replace(
        "FROM\n            supplier",
        "FROM\n            empty_supplier",
    );
    let result = sql(&empty);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"p_brand\tp_type\tp_size\tsupplier_cnt\nBrand#11\tSTANDARD\t3\t3\nBrand#12\tSTANDARD\t3\t2\nBrand#13\tSTANDARD\t3\t0\n");
    let null = query.replace(
        "FROM\n            supplier",
        "FROM\n            null_supplier",
    );
    let result = sql(&null);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"p_brand\tp_type\tp_size\tsupplier_cnt\n");
    let invalid_filter = null.replace(
        "p_brand <> 'Brand#45'",
        "missing = 1 AND p_brand <> 'Brand#45'",
    );
    assert!(!sql(&invalid_filter).status.success());
    let invalid_type = null.replace(
        "p_brand <> 'Brand#45'",
        "p_size = 'wrong' AND p_brand <> 'Brand#45'",
    );
    assert!(!sql(&invalid_type).status.success());
    let many = query.replace(
        "FROM\n            supplier",
        "FROM\n            many_supplier",
    );
    let result = sql(&many);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"p_brand\tp_type\tp_size\tsupplier_cnt\nBrand#11\tSTANDARD\t3\t3\nBrand#12\tSTANDARD\t3\t2\n");
    let disjunction = "SELECT p_brand, COUNT(DISTINCT ps_suppkey) AS n FROM partsupp s JOIN part p ON s.ps_partkey = p.p_partkey WHERE (p_brand = 'Brand#11' OR p_brand = 'Brand#12') AND ps_suppkey NOT IN (SELECT s_suppkey FROM supplier WHERE s_comment LIKE '%Customer%Complaints%') GROUP BY p_brand";
    assert!(!sql(disjunction).status.success());
    let malformed = query.replace("p_brand <> 'Brand#45'", "AND p_brand <> 'Brand#45'");
    assert!(!sql(&malformed).status.success());
    let trailing = "SELECT p_brand, COUNT(DISTINCT ps_suppkey) AS n FROM partsupp s JOIN part p ON s.ps_partkey = p.p_partkey WHERE ps_suppkey NOT IN (SELECT s_suppkey FROM supplier) AND GROUP BY p_brand";
    assert!(!sql(trailing).status.success());
    let correlated = query.replace(
        "s_comment LIKE '%Customer%Complaints%'",
        "s_suppkey = ps_suppkey",
    );
    assert!(!sql(&correlated).status.success());
    fs::remove_dir_all(db).unwrap();
}
