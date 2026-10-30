use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn empty_and_nonempty_parts_preserve_exact_count_and_sum() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-empty-part-count-{}", std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    let schema = [Column {
        name: "amount".into(),
        data_type: "UInt64".into(),
    }];
    let empty: Vec<Vec<Option<String>>> = Vec::new();
    write_wide_part(&root.join("all_1_1_0"), &schema, &empty).unwrap();
    let populated = [2u64, 5, 11]
        .into_iter()
        .map(|value| vec![Some(value.to_string())])
        .collect::<Vec<_>>();
    write_wide_part(&root.join("all_2_2_0"), &schema, &populated).unwrap();

    for (sql, expected) in [
        ("SELECT count(*) FROM part", "count\n3\n"),
        ("SELECT sum(amount) FROM part", "sum(amount)\n18\n"),
    ] {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), expected, "{sql}");
    }
    fs::remove_dir_all(root).unwrap();
}
