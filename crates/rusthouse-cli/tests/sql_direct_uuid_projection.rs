use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(statement: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn uuid_only_scan_streams_canonical_values_with_filters_and_pagination() {
    let db = std::env::temp_dir().join(format!(
        "rusthouse-direct-uuid-output-{}",
        std::process::id()
    ));
    for statement in [
        "CREATE TABLE events (uuid UUID, flag UInt8)",
        "INSERT INTO events VALUES ('00000000-0000-0000-0000-000000000001', 1), ('61f0c404-5cb3-11e7-907b-a6006ad3dba0', 0), ('ffffffff-ffff-ffff-ffff-ffffffffffff', 1)",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (query, expected) in [
        (
            "SELECT uuid FROM events",
            "uuid\n00000000-0000-0000-0000-000000000001\n61f0c404-5cb3-11e7-907b-a6006ad3dba0\nffffffff-ffff-ffff-ffff-ffffffffffff\n",
        ),
        (
            "SELECT uuid FROM events WHERE flag = 1",
            "uuid\n00000000-0000-0000-0000-000000000001\nffffffff-ffff-ffff-ffff-ffffffffffff\n",
        ),
        (
            "SELECT uuid FROM events LIMIT 1 OFFSET 1",
            "uuid\n61f0c404-5cb3-11e7-907b-a6006ad3dba0\n",
        ),
    ] {
        let result = sql(query, &db);
        assert!(result.status.success(), "{query}: {result:?}");
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
