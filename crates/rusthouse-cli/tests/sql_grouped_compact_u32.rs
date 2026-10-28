use std::fs;
use std::process::Command;

#[test]
fn thirty_two_bit_group_counts_keep_sign_and_first_seen_groups() {
    let db = std::env::temp_dir().join(format!("rusthouse-u32-groups-{}", std::process::id()));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (positive UInt32, negative Int32, flag UInt8)",
        "INSERT INTO events VALUES (4294967295, -2147483648, 1), (0, 2147483647, 0), (42, -1, 1)",
        "INSERT INTO events VALUES (4294967295, -2147483648, 1), (0, 2147483647, 1), (42, -1, 0)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (statement, expected) in [
        (
            "SELECT positive, count(*) FROM events GROUP BY positive ORDER BY positive",
            "positive\tcount\n0\t2\n42\t2\n4294967295\t2\n",
        ),
        (
            "SELECT negative, count(*) FROM events WHERE flag = 1 GROUP BY negative ORDER BY negative",
            "negative\tcount\n-2147483648\t2\n-1\t1\n2147483647\t1\n",
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
