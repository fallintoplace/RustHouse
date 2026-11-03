use std::fs;
use std::process::Command;

#[test]
fn groups_joined_dates_by_extracted_year() {
    let db = std::env::temp_dir().join(format!("rusthouse-join-year-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (id UInt64, day Nullable(Date32), amount Int64)",
        "CREATE TABLE owners (id UInt64, region String)",
        "CREATE TABLE zones (region String, label String)",
        "CREATE TABLE calendar (id UInt64, day Date)",
        "INSERT INTO events VALUES (1, '1969-12-31', 1), (2, '1970-01-01', 2), (3, '1970-12-31', 3), (4, '2000-02-29', 4), (5, NULL, 5)",
        "INSERT INTO owners VALUES (1, 'A'), (2, 'A'), (3, 'A'), (4, 'B'), (5, 'A')",
        "INSERT INTO zones VALUES ('A', 'west'), ('B', 'east')",
        "INSERT INTO calendar VALUES (1, '1970-01-01'), (2, '2000-01-01'), (3, '2000-12-31')",
    ] {
        let out = sql(statement);
        assert!(
            out.status.success(),
            "{statement}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    for (query, expected) in [
        (
            "SELECT EXTRACT(YEAR FROM e.day) AS yr, COUNT(*) AS n FROM events e JOIN owners o ON e.id = o.id GROUP BY yr ORDER BY yr NULLS LAST",
            "yr\tn\n1969\t1\n1970\t2\n2000\t1\n\\N\t1\n",
        ),
        (
            "SELECT z.label, EXTRACT(YEAR FROM e.day) AS yr, SUM(e.amount) AS total FROM events e JOIN owners o ON e.id = o.id JOIN zones z ON o.region = z.region GROUP BY z.label, yr ORDER BY z.label, yr DESC NULLS LAST",
            "z.label\tyr\ttotal\neast\t2000\t4\nwest\t1970\t5\nwest\t1969\t1\nwest\t\\N\t5\n",
        ),
        (
            "SELECT o.region, SUM(e.amount) AS total, EXTRACT(YEAR FROM e.day) AS yr FROM events e JOIN owners o ON e.id = o.id GROUP BY o.region, yr ORDER BY o.region, yr NULLS LAST",
            "o.region\ttotal\tyr\nA\t1\t1969\nA\t5\t1970\nA\t5\t\\N\nB\t4\t2000\n",
        ),
        (
            "SELECT EXTRACT(YEAR FROM c.day) AS yr, COUNT(*) AS n FROM calendar c JOIN owners o ON c.id = o.id GROUP BY yr ORDER BY yr",
            "yr\tn\n1970\t1\n2000\t2\n",
        ),
        (
            "SELECT EXTRACT(YEAR FROM e.day) AS yr, COUNT(*) AS n FROM events e JOIN owners o ON e.id = o.id GROUP BY yr ORDER BY yr DESC NULLS LAST LIMIT 1 OFFSET 1",
            "yr\tn\n1970\t2\n",
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
        "SELECT EXTRACT(YEAR FROM e.amount) AS yr, COUNT(*) FROM events e JOIN owners o ON e.id = o.id GROUP BY yr LIMIT 0",
        "SELECT EXTRACT(YEAR FROM e.missing) AS yr, COUNT(*) FROM events e JOIN owners o ON e.id = o.id GROUP BY yr LIMIT 0",
        "SELECT EXTRACT(YEAR FROM e.day) AS yr, COUNT(*) FROM events e LEFT JOIN owners o ON e.id = o.id GROUP BY yr",
        "SELECT EXTRACT(YEAR FROM e.day) AS yr, COUNT(*) FROM events e JOIN owners o ON e.id = o.id GROUP BY e.day",
        "SELECT EXTRACT(YEAR FROM e.day) AS yr, COUNT(*) FROM events e JOIN owners o ON e.id = o.id GROUP BY yr ORDER BY e.day",
        "SELECT EXTRACT(YEAR FROM e.day) AS region, COUNT(*) FROM events e JOIN owners o ON e.id = o.id GROUP BY region LIMIT 0",
        "SELECT EXTRACT(YEAR FROM e.day) AS count, COUNT(*) FROM events e JOIN owners o ON e.id = o.id GROUP BY count ORDER BY count",
        "SELECT EXTRACT(MONTH FROM e.day) AS yr, COUNT(*) FROM events e JOIN owners o ON e.id = o.id GROUP BY yr",
        "SELECT EXTRACT(YEAR FROM e.day) AS yr FROM events e JOIN owners o ON e.id = o.id",
    ] {
        assert!(!sql(query).status.success(), "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
