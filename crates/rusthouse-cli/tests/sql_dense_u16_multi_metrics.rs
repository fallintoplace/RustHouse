use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(statement: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn dense_sixteen_bit_grouped_multi_aggregates_preserve_values_and_filters() {
    let db =
        std::env::temp_dir().join(format!("rusthouse-dense-u16-multi-{}", std::process::id()));
    for statement in [
        "CREATE TABLE events (category UInt16, signed_category Int16, value UInt64, flag UInt8)",
        "INSERT INTO events VALUES (65535, -32768, 5, 1), (0, 32767, 3, 0), (42, -1, 7, 1)",
        "INSERT INTO events VALUES (65535, -32768, 9, 1), (42, -1, 1, 0), (0, 32767, 4, 1)",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (statement, expected) in [
        (
            "SELECT category, sum(value), min(value), max(value) FROM events GROUP BY category ORDER BY category",
            "category\tsum(value)\tmin(value)\tmax(value)\n0\t7\t3\t4\n42\t8\t1\t7\n65535\t14\t5\t9\n",
        ),
        (
            "SELECT signed_category, sum(value), min(value) FROM events GROUP BY signed_category ORDER BY signed_category",
            "signed_category\tsum(value)\tmin(value)\n-32768\t14\t5\n-1\t8\t1\n32767\t7\t3\n",
        ),
        (
            "SELECT category, sum(value), min(value) FROM events WHERE flag = 1 GROUP BY category ORDER BY category",
            "category\tsum(value)\tmin(value)\n0\t4\t4\n42\t7\t7\n65535\t14\t5\n",
        ),
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            expected,
            "{statement}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
