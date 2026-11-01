use std::process::Command;

fn output(sql: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", sql])
        .output()
        .expect("run RustHouse CLI")
}

#[test]
fn supports_clickhouse_base64_aliases() {
    for (sql, expected) in [
        ("SELECT to_base64('hello') AS value", "value\naGVsbG8=\n"),
        ("SELECT from_base64('aGVsbG8=') AS value", "value\nhello\n"),
        ("SELECT to_base64url('???') AS value", "value\nPz8_\n"),
        ("SELECT from_base64url('Pz8_') AS value", "value\n???\n"),
        ("SELECT from_base64(NULL) AS value", "value\n\\N\n"),
        ("SELECT to_base64url(NULL) AS value", "value\n\\N\n"),
    ] {
        let result = output(sql);
        assert!(
            result.status.success(),
            "{sql}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, expected.as_bytes(), "{sql}");
    }
}

#[test]
fn rejects_bad_alias_arity_and_padding() {
    for sql in [
        "SELECT to_base64('a', 'b')",
        "SELECT from_base64()",
        "SELECT from_base64url('a')",
    ] {
        assert!(!output(sql).status.success(), "{sql}");
    }
}
