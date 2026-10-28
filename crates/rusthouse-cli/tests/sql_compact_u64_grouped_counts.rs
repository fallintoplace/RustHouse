use std::fs;
use std::process::Command;

#[test]
fn compact_sixty_four_bit_group_counts_preserve_key_boundaries() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-compact-u64-count-{}", std::process::id()
    ));
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&db)
            .output()
            .unwrap()
    };
    for statement in [
        "CREATE TABLE events (unsigned_key UInt64, signed_key Int64, flag UInt8)",
        "INSERT INTO events VALUES (18446744073709551615, -9223372036854775807, 1), (1, 1, 0), (4294967296, -4294967296, 1)",
        "INSERT INTO events VALUES (18446744073709551615, -9223372036854775807, 1), (4294967296, -4294967296, 0), (1, 1, 1)",
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (statement, expected) in [
        (
            "SELECT unsigned_key, count(*) FROM events GROUP BY unsigned_key ORDER BY unsigned_key",
            "unsigned_key\tcount\n1\t2\n4294967296\t2\n18446744073709551615\t2\n",
        ),
        (
            "SELECT signed_key, count(*) FROM events WHERE flag = 1 GROUP BY signed_key ORDER BY signed_key",
            "signed_key\tcount\n-9223372036854775807\t2\n-4294967296\t1\n1\t1\n",
        ),
    ] {
        let result = sql(statement);
        assert!(result.status.success(), "{statement}: {result:?}");
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected, "{statement}");
    }
    fs::remove_dir_all(db).unwrap();
}
