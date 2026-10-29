use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn filtered_sorted_page_crosses_granule_and_part_boundaries() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-core-mark-sort-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let columns = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    for (index, (start, stop)) in [(0u64, 1030), (1030, 1040)].into_iter().enumerate() {
        let rows = (start..stop)
            .map(|value| vec![Some(value.to_string())])
            .collect::<Vec<_>>();
        write_wide_part(&root.join(format!("all_{index}_{index}_0")), &columns, &rows)
            .unwrap();
    }
    let sql = "SELECT id FROM part WHERE id >= 1020 ORDER BY id DESC LIMIT 8 OFFSET 4";
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
    let mut expected = String::from("id\n");
    for value in (1028..=1035).rev() {
        expected.push_str(&format!("{value}\n"));
    }
    assert_eq!(String::from_utf8(output).unwrap(), expected);
    fs::remove_dir_all(root).unwrap();
}
