use std::fs;
use std::process::Command;

#[test]
fn joined_grouped_distinct_counts_each_integer_once() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-distinct-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE part (part_id UInt64, brand String, category Nullable(String), size Int64)",
        "CREATE TABLE supplies (product UInt64, supplier Nullable(UInt64))",
        "CREATE TABLE flags (product_no UInt64, accepted UInt64)",
        "INSERT INTO part VALUES (1, 'A', 'good', 3), (2, 'A', 'good', 3), (3, 'B', 'bad type', 4), (5, 'Z', 'good', 3)",
        "INSERT INTO supplies VALUES (1, 10), (1, 10), (1, 20), (2, 10), (2, NULL), (3, 33), (5, NULL)",
        "INSERT INTO flags VALUES (1, 1), (2, 1), (3, 0), (5, 1)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let query = "SELECT brand, category, size, COUNT(DISTINCT supplier) AS n FROM part p JOIN supplies s ON p.part_id = s.product WHERE brand <> 'skip' AND category NOT LIKE 'bad%' AND size IN (3, 4) GROUP BY brand, category, size ORDER BY n DESC, brand, category, size";
    let result = sql(query);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"brand\tcategory\tsize\tn\nA\tgood\t3\t2\nZ\tgood\t3\t0\n"
    );
    let left_metric = query.replace("COUNT(DISTINCT supplier)", "COUNT(DISTINCT part_id)");
    let result = sql(&left_metric);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"brand\tcategory\tsize\tn\nA\tgood\t3\t2\nZ\tgood\t3\t1\n"
    );
    let overlap = "SELECT brand, COUNT(DISTINCT supplier) AS n FROM part p JOIN supplies s ON p.part_id = s.product WHERE size = 3 OR brand = 'A' GROUP BY brand ORDER BY brand";
    let result = sql(overlap);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"brand\tn\nA\t2\nZ\t0\n");
    let chain = query.replace(
        "ON p.part_id = s.product WHERE",
        "ON p.part_id = s.product JOIN flags f ON p.part_id = f.product_no WHERE accepted = 1 AND",
    );
    let result = sql(&chain);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"brand\tcategory\tsize\tn\nA\tgood\t3\t2\nZ\tgood\t3\t0\n"
    );
    let any = query.replace("JOIN supplies", "ANY INNER JOIN supplies");
    let result = sql(&any);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        result.stdout,
        b"brand\tcategory\tsize\tn\nA\tgood\t3\t1\nZ\tgood\t3\t0\n"
    );
    let empty = query.replace("size IN (3, 4)", "size IN (99)");
    let result = sql(&empty);
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"brand\tcategory\tsize\tn\n");
    for invalid in [
        "SELECT brand, COUNT(DISTINCT category) AS n FROM part p JOIN supplies s ON p.part_id = s.product GROUP BY brand LIMIT 0",
        "SELECT brand, COUNT(DISTINCT missing) AS n FROM part p JOIN supplies s ON p.part_id = s.product GROUP BY brand LIMIT 0",
        "SELECT brand, COUNT(DISTINCT supplier) AS n FROM part p LEFT JOIN supplies s ON p.part_id = s.product GROUP BY brand",
    ] {
        let result = sql(invalid);
        assert!(!result.status.success(), "{invalid}: {result:?}");
    }
    fs::remove_dir_all(db).unwrap();
}
