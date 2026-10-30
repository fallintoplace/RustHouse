use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(statement: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .expect("run RustHouse SQL")
}

#[test]
fn unordered_inner_join_can_hash_small_left_and_stream_large_right() {
    let database = std::env::temp_dir().join(format!(
        "rusthouse-adaptive-join-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&database);
    for statement in [
        "CREATE TABLE small (id UInt64)",
        "CREATE TABLE large (id UInt64)",
        "INSERT INTO small VALUES (39998), (39999)",
        "INSERT INTO large SELECT number FROM numbers(40000)",
    ] {
        let result = sql(statement, &database);
        assert!(result.status.success(), "{statement}: {result:?}");
    }

    let result = sql(
        "SELECT s.id, l.id FROM small s JOIN large l ON s.id = l.id LIMIT 2",
        &database,
    );
    assert!(result.status.success(), "{result:?}");
    let text = String::from_utf8(result.stdout).unwrap();
    let mut rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.remove(0), "s.id\tl.id");
    rows.sort_unstable();
    assert_eq!(rows, ["39998\t39998", "39999\t39999"]);
    fs::remove_dir_all(&database).unwrap();
}
