use std::fs;
use std::process::Command;

#[test]
fn distinct_counts_preserve_sixteen_bit_group_keys_and_null_values() {
    let db =
        std::env::temp_dir().join(format!("rusthouse-dense-group-distinct-{}", std::process::id()));
    let sql = |query: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (category UInt16, signed_key Int16, customer Nullable(String), flag UInt8)",
        "INSERT INTO events VALUES (65535, -32768, 'a', 1), (0, 32767, 'x', 1), (65535, -32768, 'b', 0)",
        "INSERT INTO events VALUES (65535, -32768, 'a', 1), (0, 32767, NULL, 1), (42, -1, NULL, 0)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (statement, expected) in [
        (
            "SELECT category, count(DISTINCT customer) FROM events GROUP BY category ORDER BY category",
            "category\tcountDistinct(customer)\n0\t1\n42\t0\n65535\t2\n",
        ),
        (
            "SELECT signed_key, count(DISTINCT customer) FROM events WHERE flag = 1 GROUP BY signed_key ORDER BY signed_key",
            "signed_key\tcountDistinct(customer)\n-32768\t1\n32767\t1\n",
        ),
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            expected,
            "{statement}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
