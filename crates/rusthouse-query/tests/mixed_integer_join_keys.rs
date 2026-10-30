use rusthouse_query::{execute_join, execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn signed_unsigned_join_keys_match_by_value_without_reinterpreting_negatives() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-mixed-join-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let a = root.join("a");
    let b = root.join("b");
    let c = root.join("c");
    write_wide_part(&a, &[Column {name:"id".into(), data_type:"Int32".into()}],
        &[-2,-1,0,1,2].map(|value| vec![Some(value.to_string())])).unwrap();
    write_wide_part(&b, &[Column {name:"id".into(), data_type:"UInt64".into()}],
        &[0,1,3].map(|value| vec![Some(value.to_string())])).unwrap();
    write_wide_part(&c, &[Column {name:"id".into(), data_type:"Int16".into()}],
        &[0,1,4].map(|value| vec![Some(value.to_string())])).unwrap();

    let query = parse_join("SELECT a.id FROM a JOIN b ON a.id = b.id ORDER BY a.id")
        .unwrap().unwrap();
    let mut result = Vec::new();
    execute_join(&query, &a, &b, &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(), "a.id\n0\n1\n");

    let query = parse_join(
        "SELECT a.id FROM a JOIN b ON a.id = b.id JOIN c ON b.id = c.id ORDER BY a.id"
    ).unwrap().unwrap();
    let mut result = Vec::new();
    execute_multi_join(&query, &[a, b, c], &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(), "a.id\n0\n1\n");
    fs::remove_dir_all(root).unwrap();
}
