use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(query: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn groups_unsigned_and_signed_16_bit_keys_across_parts() {
    let db = std::env::temp_dir().join(format!("rusthouse-dense-u16-group-{}", std::process::id()));
    for query in [
        "CREATE TABLE samples (category UInt16, signed_category Int16, flag UInt8)",
        "INSERT INTO samples VALUES (65535, -32768, 1), (0, 32767, 0), (42, -1, 1)",
        "INSERT INTO samples VALUES (65535, -32768, 1), (42, -1, 0), (0, 32767, 1)",
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT category, count(*) FROM samples GROUP BY category ORDER BY category",
            "category\tcount\n0\t2\n42\t2\n65535\t2\n",
        ),
        (
            "SELECT signed_category, count(*) FROM samples GROUP BY signed_category ORDER BY signed_category",
            "signed_category\tcount\n-32768\t2\n-1\t2\n32767\t2\n",
        ),
        (
            "SELECT category, count(*) FROM samples WHERE flag = 1 GROUP BY category ORDER BY category",
            "category\tcount\n0\t1\n42\t1\n65535\t2\n",
        ),
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            expected,
            "{query}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
