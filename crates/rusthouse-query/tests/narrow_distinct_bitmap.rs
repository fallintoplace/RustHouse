use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn exact_small_integer_distinct_counts_across_multiple_parts() {
    let root = std::env::temp_dir().join(format!("rusthouse-narrow-distinct-{}-{}", std::process::id(), line!()));
    fs::create_dir_all(&root).unwrap();
    let columns = [
        Column { name: "u8_key".into(), data_type: "UInt8".into() },
        Column { name: "i8_key".into(), data_type: "Int8".into() },
        Column { name: "u16_key".into(), data_type: "UInt16".into() },
        Column { name: "i16_key".into(), data_type: "Int16".into() },
    ];
    for part in 0..2 {
        let values = (0..2049i32).map(|index| {
            vec![
                Some((index % 251).to_string()),
                Some(((index % 241) - 120).to_string()),
                Some((index % 1999).to_string()),
                Some(((index % 2001) - 1000).to_string()),
            ]
        }).collect::<Vec<_>>();
        write_wide_part(&root.join(format!("all_{part}_{part}_0")), &columns, &values).unwrap();
    }
    for (sql, expected) in [
        ("SELECT count(DISTINCT u8_key) FROM part", "countDistinct(u8_key)\n251\n"),
        ("SELECT count(DISTINCT i8_key) FROM part", "countDistinct(i8_key)\n241\n"),
        ("SELECT count(DISTINCT u16_key) FROM part", "countDistinct(u16_key)\n1999\n"),
        ("SELECT count(DISTINCT i16_key) FROM part", "countDistinct(i16_key)\n2001\n"),
        ("SELECT count(DISTINCT u8_key) FROM part WHERE u8_key < 3", "countDistinct(u8_key)\n3\n"),
    ] {
        let mut result = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&root), &mut result).unwrap();
        assert_eq!(String::from_utf8(result).unwrap(), expected, "{sql}");
    }
    fs::remove_dir_all(root).unwrap();
}
