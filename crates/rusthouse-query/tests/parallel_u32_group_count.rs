use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn parallel_u32_grouping_merges_counts_in_part_order() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-parallel-u32-group-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let columns = [Column {name:"id".into(), data_type:"UInt32".into()}];
    for part in 0..20 {
        let rows = (0..300).map(|index| vec![Some((index % 50).to_string())])
            .collect::<Vec<_>>();
        let path = root.join(format!("p_{}_{}_0", part+1, part+1));
        write_wide_part(&path, &columns, &rows).unwrap();
    }
    let mut result = Vec::new();
    execute(&parse_select(
        "SELECT id, count(*) FROM part GROUP BY id ORDER BY id LIMIT 3"
    ).unwrap(), Some(&root), &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(),
        "id\tcount\n0\t120\n1\t120\n2\t120\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn parallel_i32_grouping_keeps_negative_keys_distinct() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-parallel-i32-group-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let columns = [Column {name:"id".into(), data_type:"Int32".into()}];
    for part in 0..20 {
        let rows = [-2, -1, 0, 1].iter()
            .map(|value| vec![Some(value.to_string())]).collect::<Vec<_>>();
        write_wide_part(
            &root.join(format!("p_{}_{}_0", part+1, part+1)),
            &columns, &rows
        ).unwrap();
    }
    let mut result = Vec::new();
    execute(&parse_select(
        "SELECT id, count(*) FROM part GROUP BY id ORDER BY id LIMIT 4"
    ).unwrap(), Some(&root), &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(),
        "id\tcount\n-2\t20\n-1\t20\n0\t20\n1\t20\n");
    fs::remove_dir_all(root).unwrap();
}
