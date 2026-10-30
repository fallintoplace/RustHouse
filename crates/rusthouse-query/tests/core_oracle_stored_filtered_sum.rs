use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn filtered_sum_and_count_cover_every_boundary_row_once() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-core-filtered-sum-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let columns = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    for (index, (start, stop)) in [(0u64, 1025), (1025, 1042)].into_iter().enumerate() {
        let rows = (start..stop)
            .map(|value| vec![Some(value.to_string())])
            .collect::<Vec<_>>();
        write_wide_part(&root.join(format!("all_{index}_{index}_0")), &columns, &rows)
            .unwrap();
    }
    let sql = "SELECT sum(id), count(*) FROM part WHERE id >= 1019";
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
    let expected_sum: u64 = (1019..=1041).sum();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        format!("sum(id)\tcount(*)\n{expected_sum}\t23\n")
    );
    fs::remove_dir_all(root).unwrap();
}
