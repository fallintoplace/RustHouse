use std::fs;
use std::process::Command;

#[test]
fn prewhere_filters_persisted_rows_and_combines_with_where() {
    let path = std::env::temp_dir().join(format!("rusthouse-prewhere-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    let sql = |statement: &str| {
        Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", statement, "--database"])
            .arg(&path)
            .output()
            .unwrap()
    };
    let created = sql("CREATE TABLE events (id UInt64, category String)");
    assert!(created.status.success(), "{created:?}");
    let inserted = sql("INSERT INTO events VALUES (1, 'a'), (2, 'b'), (3, 'a'), (4, 'b')");
    assert!(inserted.status.success(), "{inserted:?}");
    let selected = sql("SELECT id, category FROM events PREWHERE id >= 2 WHERE category = 'b' ORDER BY id");
    assert!(selected.status.success(), "{selected:?}");
    assert_eq!(selected.stdout, b"id\tcategory\n2\tb\n4\tb\n");
    fs::remove_dir_all(path).unwrap();
}
