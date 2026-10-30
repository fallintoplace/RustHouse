use std::process::Command;

#[test]
fn supports_initcap_utf8_constants() {
    for (query, expected) in [
        ("SELECT initcapUTF8('hELLO wORLD') AS value", "value\nHello World\n"),
        ("SELECT initcapUTF8('éCLAIR MÜNCHEN') AS value", "value\nÉclair München\n"),
        ("SELECT initcapUTF8('') AS value", "value\n\n"),
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
    let query = "SELECT initcapUTF8()";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
