use std::fs;
use std::process::Command;

#[test]
fn qualified_boolean_filters_bind_each_join_alias() {
    let db =
        std::env::temp_dir().join(format!("rusthouse-qualified-filter-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE trips (id UInt64, from_id UInt64, to_id UInt64)",
        "CREATE TABLE nation (nid UInt64, name Nullable(String))",
        "INSERT INTO trips VALUES (1, 10, 20), (2, 20, 10), (3, 10, 30), (4, 30, 10), (5, 10, 10)",
        "INSERT INTO nation VALUES (10, 'FRANCE'), (20, 'GERMANY'), (30, NULL)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT trips.id FROM trips JOIN nation n1 ON trips.from_id = n1.nid JOIN nation n2 ON trips.to_id = n2.nid WHERE (n1.name = 'FRANCE' AND n2.name = 'GERMANY') OR (n1.name = 'GERMANY' AND n2.name = 'FRANCE') ORDER BY trips.id",
            "trips.id\n1\n2\n",
        ),
        (
            "SELECT trips.id FROM trips JOIN nation n1 ON trips.from_id = n1.nid JOIN nation n2 ON trips.to_id = n2.nid WHERE n1.name LIKE 'F%' AND n2.name IS NOT NULL ORDER BY trips.id DESC LIMIT 1 OFFSET 1",
            "trips.id\n1\n",
        ),
        (
            "SELECT COUNT(*) AS n FROM trips JOIN nation n1 ON trips.from_id = n1.nid JOIN nation n2 ON trips.to_id = n2.nid WHERE n1.name = n2.name OR n2.name IS NULL",
            "n\n2\n",
        ),
        (
            "SELECT n1.nid, COUNT(*) AS total FROM trips JOIN nation n1 ON trips.from_id = n1.nid JOIN nation n2 ON trips.to_id = n2.nid WHERE n1.name = 'FRANCE' OR n2.name = 'FRANCE' GROUP BY n1.nid ORDER BY n1.nid",
            "n1.nid\ttotal\n10\t3\n20\t1\n30\t1\n",
        ),
        (
            "SELECT trips.id FROM trips JOIN nation n1 ON trips.from_id = n1.nid JOIN nation n2 ON trips.to_id = n2.nid WHERE \"n1\".\"name\" ILIKE 'fr%' OR n2.name IS NULL ORDER BY trips.id",
            "trips.id\n1\n3\n5\n",
        ),
        (
            "SELECT trips.id FROM trips, nation n1, nation n2 WHERE trips.from_id = n1.nid AND trips.to_id = n2.nid AND (n1.name = 'FRANCE' OR n2.name = 'FRANCE') ORDER BY trips.id",
            "trips.id\n1\n2\n3\n4\n5\n",
        ),
    ] {
        let result = sql(query);
        assert!(
            result.status.success(),
            "{query}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, expected.as_bytes(), "{query}");
    }
    for query in [
        "SELECT trips.id FROM trips JOIN nation n1 ON trips.from_id = n1.nid JOIN nation n2 ON trips.to_id = n2.nid WHERE name LIKE 'F%' LIMIT 0",
        "SELECT trips.id FROM trips JOIN nation n1 ON trips.from_id = n1.nid JOIN nation n2 ON trips.to_id = n2.nid WHERE missing.name LIKE 'F%' LIMIT 0",
        "SELECT trips.id FROM trips JOIN nation n1 ON trips.from_id = n1.nid JOIN nation n2 ON trips.to_id = n2.nid WHERE n1.nid LIKE 'F%' LIMIT 0",
        "SELECT trips.id FROM trips LEFT JOIN nation n1 ON trips.from_id = n1.nid JOIN nation n2 ON trips.to_id = n2.nid WHERE n1.name = 'FRANCE' OR n2.name = 'GERMANY'",
    ] {
        assert!(!sql(query).status.success(), "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
