use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(query: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn left_join_grouped_count_preserves_unmatched_rows_and_null_keys() {
    let db = std::env::temp_dir().join(format!("rusthouse-left-group-count-{}", std::process::id()));
    for statement in [
        "CREATE TABLE rides (id UInt64, pickup Nullable(UInt8))",
        "CREATE TABLE zones (id UInt64, region Nullable(String))",
        "CREATE TABLE empty_zones (id UInt64, region String)",
        "INSERT INTO rides VALUES (1, 10), (1, 10), (2, 20), (3, 30), (4, NULL), (5, 20), (6, 20)",
        "INSERT INTO zones VALUES (1, 'EU'), (1, NULL), (2, 'US'), (4, 'EU'), (99, 'other')",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }

    let cases: &[(&str, &[u8])] = &[
        (
            "SELECT r.pickup, COUNT(*) FROM rides r LEFT JOIN zones z ON r.id = z.id GROUP BY r.pickup ORDER BY r.pickup ASC NULLS LAST",
            b"r.pickup\tcount\n10\t4\n20\t3\n30\t1\n\\N\t1\n",
        ),
        (
            "SELECT z.region, COUNT(*) FROM rides r LEFT JOIN zones z ON r.id = z.id GROUP BY z.region ORDER BY z.region ASC NULLS LAST",
            b"z.region\tcount\nEU\t3\nUS\t1\n\\N\t5\n",
        ),
        (
            "SELECT z.region, COUNT(*) FROM rides r ANY LEFT JOIN zones z ON r.id = z.id GROUP BY z.region ORDER BY z.region ASC NULLS LAST",
            b"z.region\tcount\nEU\t3\nUS\t1\n\\N\t3\n",
        ),
        (
            "SELECT z.region, COUNT(*) FROM rides r LEFT JOIN zones z ON r.id = z.id WHERE z.region IS NULL GROUP BY z.region",
            b"z.region\tcount\n\\N\t5\n",
        ),
        (
            "SELECT z.region, COUNT(*) FROM rides r LEFT JOIN zones z ON r.id = z.id WHERE z.region IS NOT NULL GROUP BY z.region ORDER BY z.region",
            b"z.region\tcount\nEU\t3\nUS\t1\n",
        ),
        (
            "SELECT z.region, COUNT(*) FROM rides r LEFT JOIN zones z ON r.id = z.id WHERE r.pickup IS NULL GROUP BY z.region",
            b"z.region\tcount\nEU\t1\n",
        ),
        (
            "SELECT e.region, COUNT(*) FROM rides r LEFT JOIN empty_zones e ON r.id = e.id GROUP BY e.region",
            b"e.region\tcount\n\\N\t7\n",
        ),
        (
            "SELECT z.region, COUNT(*) FROM rides r LEFT JOIN zones z ON r.id = z.id GROUP BY z.region LIMIT 0",
            b"z.region\tcount\n",
        ),
    ];
    for &(statement, expected) in cases {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
        assert_eq!(result.stdout, expected, "{statement}");
    }

    for statement in [
        "SELECT z.region, COUNT(*) FROM rides r RIGHT JOIN zones z ON r.id = z.id GROUP BY z.region",
        "SELECT z.region, COUNT(*) FROM rides r FULL JOIN zones z ON r.id = z.id GROUP BY z.region",
    ] {
        assert!(!sql(statement, &db).status.success(), "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}
