use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn constant_physical_histogram_has_one_exact_bucket() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-perf-hist-constant-{}", std::process::id(),
    ));
    let columns = [Column { name: "value".into(), data_type: "UInt64".into() }];
    let rows = vec![vec![Some("42".into())]; 1024];
    write_wide_part(&path, &columns, &rows).unwrap();
    let query = parse_select("SELECT histogram(32)(value) FROM part").unwrap();
    let mut output = Vec::new();
    execute(&query, Some(&path), &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "histogram(32)(value)\n[(42,42,1024)]\n",
    );
    fs::remove_dir_all(&path).unwrap();
}
