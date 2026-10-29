use std::process::Command;

#[test]
fn decodes_padded_and_unpadded_url_safe_base64() {
    for (query, expected) in [
        ("SELECT base64URLDecode('') AS value", "value\n\n"),
        ("SELECT base64URLDecode('Zg') AS value", "value\nf\n"),
        ("SELECT base64URLDecode('Zm8') AS value", "value\nfo\n"),
        ("SELECT base64URLDecode('Zm9v') AS value", "value\nfoo\n"),
        ("SELECT base64URLDecode('8J-Mng') AS value", "value\n🌞\n"),
        ("SELECT base64URLDecode('8J-Mng==') AS value", "value\n🌞\n"),
        ("SELECT base64URLDecode(NULL) AS value", "value\n\\N\n"),
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
fn rejects_malformed_url_safe_base64() {
    for query in [
        "SELECT base64URLDecode()",
        "SELECT base64URLDecode('a')",
        "SELECT base64URLDecode('%%%%')",
        "SELECT base64URLDecode('ab+/')",
        "SELECT base64URLDecode('Zg=', 'other')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
