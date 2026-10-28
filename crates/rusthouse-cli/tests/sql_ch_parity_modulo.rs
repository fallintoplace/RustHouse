use std::process::Command;

#[test]
fn clickhouse_modulo_constants() {
    for (query, expected) in [
        ("SELECT modulo(-7, 3) AS value", "value\n-1\n"),
        ("SELECT modulo(7, -3) AS value", "value\n1\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .expect("start rusthouse");
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}
