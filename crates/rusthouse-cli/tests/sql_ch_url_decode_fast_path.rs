use std::process::Command;

#[test]
fn url_decoders_keep_plain_paths_and_transform_escapes() {
    for (query, expected) in [
        ("SELECT decodeURLComponent('path/hello?a=1&b=2') AS value", "value\npath/hello?a=1&b=2\n"),
        ("SELECT decodeURLComponent('a+b') AS value", "value\na+b\n"),
        ("SELECT decodeURLFormComponent('a+b') AS value", "value\na b\n"),
        ("SELECT decodeURLFormComponent('a%2Bb') AS value", "value\na+b\n"),
        ("SELECT URL_DECODE('a%20b') AS value", "value\na b\n"),
        ("SELECT URL_DECODE(NULL) AS value", "value\n\\N\n"),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse")).args(["sql", query]).output().unwrap();
        assert!(result.status.success(), "{query}: {}", String::from_utf8_lossy(&result.stderr));
        assert_eq!(result.stdout, expected.as_bytes(), "{query}");
    }
}
