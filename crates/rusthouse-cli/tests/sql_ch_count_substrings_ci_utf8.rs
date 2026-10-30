use std::process::Command;

#[test]
fn supports_count_substrings_ci_utf8_constants() {
    for (query, expected) in [
        ("SELECT countSubstringsCaseInsensitiveUTF8('ÄxäXÄx', 'äx') AS value", "value\n3\n"),
        ("SELECT countSubstringsCaseInsensitiveUTF8('ÄxäXÄx', 'äx', 3) AS value", "value\n2\n"),
        ("SELECT countSubstringsCaseInsensitiveUTF8('abc', 'Z') AS value", "value\n0\n"),
        ("SELECT countSubstringsCaseInsensitiveUTF8('aaa', '') AS value", "value\n0\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{query}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
    let query = "SELECT countSubstringsCaseInsensitiveUTF8('x')";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
