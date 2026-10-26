use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn inspects_part_as_json_without_changing_default_output() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../rusthouse-storage/tests/fixtures/all_1_1_0");

    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("inspect-part")
        .arg(&path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let expected = format!(
        "{{\"path\":\"{}\",\"rows\":42,\"columns\":[{{\"name\":\"id\",\"type\":\"UInt64\"}},{{\"name\":\"city\",\"type\":\"LowCardinality(String)\"}}]}}\n",
        path.display()
    );
    assert_eq!(result.stdout, expected.as_bytes());

    let default = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("inspect-part")
        .arg(&path)
        .output()
        .unwrap();
    assert!(default.status.success(), "{default:?}");
    assert!(String::from_utf8_lossy(&default.stdout).contains("Columns (2):"));
}

#[test]
fn escapes_column_names_loaded_from_metadata() {
    let dir = std::env::temp_dir().join(format!("rusthouse-inspect-json-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("count.txt"), "1\n").unwrap();
    fs::write(
        dir.join("columns.txt"),
        "columns format version: 1\n1 columns:\n`quote\\\"and\\\\path` String\n",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("inspect-part")
        .arg(&dir)
        .arg("--json")
        .output()
        .unwrap();
    fs::remove_dir_all(dir).unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(String::from_utf8_lossy(&result.stdout).contains(r#""name":"quote\"and\\path""#));
}

#[test]
fn rejects_unknown_extra_or_missing_arguments() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../rusthouse-storage/tests/fixtures/all_1_1_0");
    for arguments in [vec!["--bad"], vec!["--json", "--json"]] {
        let result = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .arg("inspect-part")
            .arg(&path)
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2), "{result:?}");
    }
    let missing = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["inspect-part", "--json"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
}
