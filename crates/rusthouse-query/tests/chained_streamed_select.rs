use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn chained_inner_select_with_limit_does_not_materialize_fact_join_product() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-stream-select-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let schema = [Column { name: "id".into(), data_type: "UInt64".into() }];
    let paths = ["a", "b", "c"].map(|name| root.join(name));
    let fact = (0..70_000).map(|_| vec![Some("7".into())]).collect::<Vec<_>>();
    write_wide_part(&paths[0], &schema, &fact).unwrap();
    write_wide_part(&paths[1], &schema, &[
        vec![Some("7".into())], vec![Some("7".into())],
    ]).unwrap();
    write_wide_part(&paths[2], &schema, &[
        vec![Some("7".into())], vec![Some("7".into())],
        vec![Some("7".into())],
    ]).unwrap();

    let query = parse_join(
        "SELECT a.id FROM a JOIN b ON a.id = b.id JOIN c ON b.id = c.id LIMIT 3 OFFSET 4"
    ).unwrap().unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &paths, &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "a.id\n7\n7\n7\n");

    let query = parse_join(
        "SELECT * FROM a ANY JOIN b ON a.id = b.id ANY JOIN c ON b.id = c.id LIMIT 1"
    ).unwrap().unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &paths, &mut output).unwrap();
    assert!(String::from_utf8(output).unwrap().ends_with("7\t7\t7\n"));
    fs::remove_dir_all(root).unwrap();
}
