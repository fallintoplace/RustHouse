use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn eight_bit_distinct_uses_correct_signed_key_space() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-perf-distinct-int8-{}", std::process::id(),
    ));
    let columns = [Column { name: "n".into(), data_type: "Int8".into() }];
    let rows = [-128, -1, 0, 1, -128, -1, 1].into_iter()
        .map(|value| vec![Some(value.to_string())])
        .collect::<Vec<_>>();
    write_wide_part(&path, &columns, &rows).unwrap();
    let mut output = Vec::new();
    execute(&parse_select("SELECT count(DISTINCT n) FROM part").unwrap(), Some(&path), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "countDistinct(n)\n4\n");
    let mut output = Vec::new();
    execute(&parse_select("SELECT DISTINCT n FROM part").unwrap(), Some(&path), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "n\n-128\n-1\n0\n1\n");
    fs::remove_dir_all(&path).unwrap();
}
