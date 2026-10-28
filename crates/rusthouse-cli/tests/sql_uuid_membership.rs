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
fn uuid_in_and_not_in_keep_exact_byte_semantics() {
    let db = std::env::temp_dir().join(format!("rusthouse-uuid-in-{}", std::process::id()));
    for statement in [
        "CREATE TABLE events (id UInt64, user_id UUID)",
        "INSERT INTO events VALUES (1, '00000000-0000-0000-0000-000000000001'), (2, '00000000-0000-0000-0000-000000000002'), (3, '00000000-0000-0000-0000-000000000003')",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    let included = sql(
        "SELECT id FROM events WHERE user_id IN ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000003') ORDER BY id",
        &db,
    );
    assert!(included.status.success(), "{included:?}");
    assert_eq!(included.stdout, b"id\n1\n3\n");
    let excluded = sql(
        "SELECT id FROM events WHERE user_id NOT IN ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000003') ORDER BY id",
        &db,
    );
    assert!(excluded.status.success(), "{excluded:?}");
    assert_eq!(excluded.stdout, b"id\n2\n");
    let invalid = sql(
        "SELECT id FROM events WHERE user_id IN ('invalid')",
        &db,
    );
    assert!(!invalid.status.success());
    fs::remove_dir_all(db).unwrap();
}
