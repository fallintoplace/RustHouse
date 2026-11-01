use std::process::Command;

#[test]
fn trailing_character_is_appended_when_input_is_empty() {
    for (query, expected) in [
        (
            "SELECT appendTrailingCharIfAbsent('', '/') AS value",
            "value\n/\n",
        ),
        (
            "SELECT appendTrailingCharIfAbsent('hello', '/') AS value",
            "value\nhello/\n",
        ),
        (
            "SELECT appendTrailingCharIfAbsent('hello/', '/') AS value",
            "value\nhello/\n",
        ),
        (
            "SELECT appendTrailingCharIfAbsent('', '✓') AS value",
            "value\n✓\n",
        ),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{query}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, expected.as_bytes(), "{query}");
    }
}
