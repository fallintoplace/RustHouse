use std::process::Command;

#[test]
fn validates_edit_distance_utf8_results() {
    for (query, expected) in [
        ("SELECT editDistanceUTF8('kitten','sitting') AS value", "value\n3\n"),
        ("SELECT editDistanceUTF8('é','e') AS value", "value\n1\n"),
        ("SELECT editDistanceUTF8('你好','你们') AS value", "value\n1\n"),
        ("SELECT editDistanceUTF8('🌞','🌛') AS value", "value\n1\n"),
        ("SELECT editDistanceUTF8('','abc') AS value", "value\n3\n"),
        ("SELECT editDistanceUTF8(NULL,'a') AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .expect("execute RustHouse SQL");
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}

#[test]
fn rejects_edit_distance_utf8_bad_arguments() {
    for query in [
        "SELECT editDistanceUTF8()",
        "SELECT editDistanceUTF8('a')",
        "SELECT editDistanceUTF8('a','b','c')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
