//! Persisted physical-part query correctness regression.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::Path;

fn run(root: &Path, sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(root), &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn physical_multikey_revenue() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-physical_multikey_revenue-{}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();

    let schema = [
        Column {
            name: "region".into(),
            data_type: "String".into(),
        },
        Column {
            name: "category".into(),
            data_type: "String".into(),
        },
        Column {
            name: "amount".into(),
            data_type: "UInt64".into(),
        },
    ];
    let parts: Vec<Vec<Vec<Option<String>>>> = vec![
        vec![
            vec![Some("EU".into()), Some("books".into()), Some("7".into())],
            vec![Some("NA".into()), Some("games".into()), Some("12".into())],
            vec![Some("EU".into()), Some("books".into()), Some("3".into())],
        ],
        vec![
            vec![Some("EU".into()), Some("games".into()), Some("2".into())],
            vec![Some("NA".into()), Some("games".into()), Some("6".into())],
            vec![Some("EU".into()), Some("books".into()), Some("5".into())],
        ],
    ];
    for (index, rows) in parts.iter().enumerate() {
        let part = root.join(format!("all_{index}_{index}_0"));
        write_wide_part(&part, &schema, rows).unwrap();
    }

    let sql = "SELECT region, category, sum(amount) FROM part GROUP BY region, category ORDER BY region, category";
    let expected = "region\tcategory\tsum(amount)\nEU\tbooks\t15\nEU\tgames\t2\nNA\tgames\t18\n";
    assert_eq!(run(&root, sql), expected, "{sql}");
    fs::remove_dir_all(root).unwrap();
}
