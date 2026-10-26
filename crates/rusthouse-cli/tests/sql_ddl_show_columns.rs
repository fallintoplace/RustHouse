use std::fs;
use std::process::Command;

fn run(query: &str, mode: &str, dir: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, mode])
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn show_columns_and_desc_return_the_same_schema_in_catalog_mode() {
    let dir = std::env::temp_dir().join(format!("rusthouse-show-columns-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    assert!(
        run(
            "CREATE TABLE events (id UInt64, label Nullable(String))",
            "--catalog",
            &dir,
        )
        .status
        .success()
    );
    let original = run("DESCRIBE TABLE events", "--catalog", &dir);
    assert!(original.status.success());
    for command in ["SHOW COLUMNS FROM events", "DESC TABLE events"] {
        let result = run(command, "--catalog", &dir);
        assert!(result.status.success(), "{result:?}");
        assert_eq!(result.stdout, original.stdout);
    }
    fs::remove_dir_all(dir).unwrap();
}
