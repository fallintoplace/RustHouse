use std::fs;
use std::process::Command;

#[test]
fn grouped_join_keys_accept_output_aliases() {
    let db = std::env::temp_dir().join(format!("rusthouse-group-alias-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (id UInt64, day Date, amount Int64)",
        "CREATE TABLE owners (id UInt64, region Nullable(String))",
        "CREATE TABLE zones (region Nullable(String), label String)",
        "INSERT INTO events VALUES (1, '1970-01-01', 2), (2, '1970-12-31', 3), (3, '2000-02-29', 4)",
        "INSERT INTO owners VALUES (1, 'A'), (2, 'A'), (3, 'B')",
        "INSERT INTO zones VALUES ('A', 'west'), ('B', 'east')",
    ] {
        let result = sql(statement);
        assert!(
            result.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    for (query, expected) in [
        (
            "SELECT o.region AS area, COUNT(*) AS n FROM events e JOIN owners o ON e.id = o.id GROUP BY area ORDER BY area",
            "area\tn\nA\t2\nB\t1\n",
        ),
        (
            "SELECT z.label AS nation, EXTRACT(YEAR FROM e.day) AS yr, SUM(e.amount) AS revenue FROM events e JOIN owners o ON e.id = o.id JOIN zones z ON o.region = z.region GROUP BY nation, yr ORDER BY nation, yr DESC",
            "nation\tyr\trevenue\neast\t2000\t4\nwest\t1970\t5\n",
        ),
        (
            "SELECT o.region AS area, COUNT(*) AS n FROM events e JOIN owners o ON e.id = o.id GROUP BY o.region ORDER BY area DESC LIMIT 1",
            "area\tn\nB\t1\n",
        ),
        (
            "SELECT o.region AS area, COUNT(*) AS n FROM events e JOIN owners o ON e.id = o.id GROUP BY area ORDER BY area LIMIT 0",
            "area\tn\n",
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
        "SELECT o.region AS id, COUNT(*) FROM events e JOIN owners o ON e.id = o.id GROUP BY id LIMIT 0",
        "SELECT o.region AS count, COUNT(*) FROM events e JOIN owners o ON e.id = o.id GROUP BY count",
        "SELECT o.region AS area, e.id AS area, COUNT(*) FROM events e JOIN owners o ON e.id = o.id GROUP BY area, e.id",
        "SELECT o.region AS yr, EXTRACT(YEAR FROM e.day) AS yr, COUNT(*) FROM events e JOIN owners o ON e.id = o.id GROUP BY yr, e.day",
        "SELECT o.region AS area, COUNT(*) FROM events e LEFT JOIN owners o ON e.id = o.id GROUP BY area",
        "SELECT o.region AS area FROM events e JOIN owners o ON e.id = o.id",
    ] {
        assert!(!sql(query).status.success(), "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
