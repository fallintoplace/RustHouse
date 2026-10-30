use std::process::Command;

fn sql(query: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .expect("run query")
}

#[test]
fn decoders_ignore_permitted_base64_whitespace() {
    for (query, expected) in [
        ("SELECT base64Decode(' Zm8= ') AS value", "value\nfo\n"),
        ("SELECT base64URLDecode(' Pz8_ ') AS value", "value\n???\n"),
        ("SELECT base64Decode('Z m 8 =') AS value", "value\nfo\n"),
        ("SELECT tryBase64Decode(' Zm8= ') AS value", "value\nfo\n"),
    ] {
        let output = sql(query);
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}

#[test]
fn decoders_still_reject_invalid_non_whitespace_bytes() {
    for query in [
        "SELECT base64Decode('Zm8=~')",
        "SELECT base64URLDecode('Zm8=@')",
    ] {
        assert!(!sql(query).status.success(), "{query}");
    }
}
