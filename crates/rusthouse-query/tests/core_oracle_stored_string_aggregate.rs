use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn grouped_string_keys_merge_all_immutable_part_contributions() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-core-string-aggregate-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let columns = [
        Column {
            name: "dept".into(),
            data_type: "String".into(),
        },
        Column {
            name: "amount".into(),
            data_type: "UInt64".into(),
        },
    ];
    let parts = [
        [("east", "1"), ("west", "5"), ("east", "3")],
        [("west", "7"), ("east", "10"), ("west", "0")],
    ];
    for (index, part) in parts.iter().enumerate() {
        let rows = part
            .iter()
            .map(|(dept, amount)| vec![Some((*dept).into()), Some((*amount).into())])
            .collect::<Vec<_>>();
        write_wide_part(&root.join(format!("all_{index}_{index}_0")), &columns, &rows)
            .unwrap();
    }
    let sql = "SELECT dept, sum(amount) FROM part GROUP BY dept ORDER BY dept";
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "dept\tsum(amount)\neast\t14\nwest\t12\n"
    );
    fs::remove_dir_all(root).unwrap();
}
