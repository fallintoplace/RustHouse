use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn external_u32_group_by_supports_more_than_two_hundred_thousand_keys() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-external-u32-group-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let schema = [Column { name:"id".into(), data_type:"UInt32".into() }];
    let rows = (0..220_100u32).map(|id| vec![Some(id.to_string())])
        .collect::<Vec<_>>();
    write_wide_part(&root, &schema, &rows).unwrap();
    let sql = "SELECT id, count(*) FROM part GROUP BY id ORDER BY id ASC LIMIT 3 OFFSET 200000";
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(),
        "id\tcount\n200000\t1\n200001\t1\n200002\t1\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn negative_i32_keys_sort_correctly_through_external_grouped_runs() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-external-i32-group-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let schema = [Column { name:"id".into(), data_type:"Int32".into() }];
    let values = [-100, -1, 0, 1, 17, i32::MIN, i32::MAX];
    let rows = values.iter().map(|id| vec![Some(id.to_string())])
        .collect::<Vec<_>>();
    write_wide_part(&root, &schema, &rows).unwrap();
    let sql = "SELECT id, count(*) FROM part GROUP BY id ORDER BY id ASC LIMIT 3";
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(),
        "id\tcount\n-2147483648\t1\n-100\t1\n-1\t1\n");
    fs::remove_dir_all(root).unwrap();
}
