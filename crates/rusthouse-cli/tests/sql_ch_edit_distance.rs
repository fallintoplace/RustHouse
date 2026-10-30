use std::process::Command;

#[test]
fn validates_edit_distance_results() {
    for (query, expected) in [
        ("SELECT editDistance('kitten','sitting') AS value", "value\n3\n"),
        ("SELECT editDistance('clickhouse','mouse') AS value", "value\n6\n"),
        ("SELECT editDistance('','abc') AS value", "value\n3\n"),
        ("SELECT editDistance('a','') AS value", "value\n1\n"),
        ("SELECT editDistance('same','same') AS value", "value\n0\n"),
        ("SELECT editDistance('é','e') AS value", "value\n2\n"),
        ("SELECT editDistance(NULL,'a') AS value", "value\n\\N\n"),
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
fn rejects_edit_distance_bad_arguments() {
    for query in [
        "SELECT editDistance()",
        "SELECT editDistance('a')",
        "SELECT editDistance('a','b','c')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
