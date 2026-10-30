use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn distinct_string_values_are_deduplicated_across_parts() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-core-distinct-text-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let columns = [Column {
        name: "grp".into(),
        data_type: "String".into(),
    }];
    let parts = [["red", "blue", "blue"], ["red", "green", "blue"]];
    for (index, part) in parts.iter().enumerate() {
        let rows = part
            .iter()
            .map(|value| vec![Some((*value).into())])
            .collect::<Vec<_>>();
        write_wide_part(&root.join(format!("all_{index}_{index}_0")), &columns, &rows)
            .unwrap();
    }
    let sql = "SELECT DISTINCT grp FROM part ORDER BY grp";
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "grp\nblue\ngreen\nred\n");
    fs::remove_dir_all(root).unwrap();
}
