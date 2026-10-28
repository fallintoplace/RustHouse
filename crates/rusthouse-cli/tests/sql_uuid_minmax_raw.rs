use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(query: &str, db: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--database"])
        .arg(db)
        .output()
        .unwrap()
}

#[test]
fn uuid_extrema_keep_canonical_order_across_parts_and_filters() {
    let db = std::env::temp_dir().join(format!("rusthouse-uuid-minmax-{}", std::process::id()));
    for query in [
        "CREATE TABLE records (value UUID, flag UInt8)",
        "INSERT INTO records VALUES ('00000000-0000-0000-0000-0000000000ff', 0), ('00000000-0000-0000-0000-000000000001', 1)",
        "INSERT INTO records VALUES ('ffffffff-ffff-ffff-ffff-ffffffffffff', 1), ('00000000-0000-0000-0000-000000000002', 1)",
    ] {
        let output = sql(query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
    }
    for (query, expected) in [
        ("SELECT min(value) FROM records", "min(value)\n00000000-0000-0000-0000-000000000001\n"),
        ("SELECT max(value) FROM records", "max(value)\nffffffff-ffff-ffff-ffff-ffffffffffff\n"),
        ("SELECT min(value) FROM records WHERE flag = 0", "min(value)\n00000000-0000-0000-0000-0000000000ff\n"),
    ] {
        let output = sql(query, &db);
        assert!(output.status.success(), "{query}: {output:?}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected, "{query}");
    }
    fs::remove_dir_all(db).unwrap();
}
