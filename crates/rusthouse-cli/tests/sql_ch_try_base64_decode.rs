use std::process::Command;

#[test]
fn try_base64_decode_decodes_expected_values() {
    for (query, expected) in [
        ("SELECT tryBase64Decode('Y2xpY2tob3VzZQ==') AS value", "value\nclickhouse\n"),
        ("SELECT tryBase64Decode('Zm9v') AS value", "value\nfoo\n"),
        ("SELECT tryBase64Decode('invalid') AS value", "value\n\n"),
        ("SELECT tryBase64Decode('%%%') AS value", "value\n\n"),
        ("SELECT tryBase64Decode('8J+Mng==') AS value", "value\n🌞\n"),
        ("SELECT tryBase64Decode(NULL) AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}

#[test]
fn try_base64_decode_rejects_bad_arity() {
    for query in [
        "SELECT tryBase64Decode()",
        "SELECT tryBase64Decode('a','b')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
