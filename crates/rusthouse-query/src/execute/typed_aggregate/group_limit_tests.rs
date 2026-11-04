use super::*;
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn grouped_aggregate_accepts_200000_groups_and_rejects_one_more() {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-group-limit-{}-{}",
        std::process::id(),
        line!(),
    ));
    let _ = fs::remove_dir_all(&part);
    let columns = [
        Column {
            name: "group_key".into(),
            data_type: "UInt32".into(),
        },
        Column {
            name: "amount".into(),
            data_type: "UInt64".into(),
        },
    ];
    let rows = (0..=MAX_GROUPS)
        .map(|key| vec![Some(key.to_string()), Some("1".into())])
        .collect::<Vec<_>>();
    write_wide_part(&part, &columns, &rows).unwrap();

    let query = crate::parse_select(&format!(
        "SELECT group_key, sum(amount) FROM part WHERE group_key < {MAX_GROUPS} GROUP BY group_key"
    ))
    .unwrap();
    let mut output = Vec::new();
    crate::execute(&query, Some(&part), &mut output).unwrap();
    assert_eq!(output.split(|byte| *byte == b'\n').count(), MAX_GROUPS + 2);

    let query =
        crate::parse_select("SELECT group_key, sum(amount) FROM part GROUP BY group_key").unwrap();
    let error = crate::execute(&query, Some(&part), &mut Vec::new()).unwrap_err();
    assert!(
        error.to_string().contains("at most 200000 groups"),
        "{error}"
    );

    fs::remove_dir_all(part).unwrap();
}
