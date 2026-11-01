use std::process::Command;

#[test]
fn verifies_translate_ascii_contract_semantics() {
    for (query, expected) in [
        (
            "SELECT translate('café', 'af', 'AF') AS value",
            "value\ncAFé\n",
        ),
        (
            "SELECT translateUTF8('café', 'é', 'e') AS value",
            "value\ncafe\n",
        ),
        (
            "SELECT translate('hello', 'eo', 'EO') AS value",
            "value\nhEllO\n",
        ),
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
    let query = "SELECT translate('café', 'é', 'e')";
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{query} unexpectedly succeeded");
}
