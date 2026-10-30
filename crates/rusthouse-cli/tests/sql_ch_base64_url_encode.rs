use std::process::Command;

#[test]
fn base64_url_encode_reports_correct_values() {
    for (query, expected) in [
        ("SELECT base64URLEncode('') AS value", "value\n\n"),
        ("SELECT base64URLEncode('f') AS value", "value\nZg\n"),
        ("SELECT base64URLEncode('fo') AS value", "value\nZm8\n"),
        ("SELECT base64URLEncode('foo') AS value", "value\nZm9v\n"),
        ("SELECT base64URLEncode('🌞') AS value", "value\n8J-Mng\n"),
        ("SELECT base64URLEncode(NULL) AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .expect("run SQL");
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}

#[test]
fn base64_url_encode_rejects_bad_inputs() {
    for query in [
        "SELECT base64URLEncode()",
        "SELECT base64URLEncode('a','b')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
