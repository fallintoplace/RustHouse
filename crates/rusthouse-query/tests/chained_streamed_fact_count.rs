use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn chained_count_streams_large_fact_table_past_intermediate_row_limits() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-streamed-fact-count-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let schema = [Column { name: "id".into(), data_type: "UInt64".into() }];
    let fact = root.join("fact");
    let dim1 = root.join("dim1");
    let dim2 = root.join("dim2");
    let facts = (0..70_000).map(|_| vec![Some("7".into())]).collect::<Vec<_>>();
    write_wide_part(&fact, &schema, &facts).unwrap();
    write_wide_part(&dim1, &schema, &[
        vec![Some("7".into())], vec![Some("7".into())],
    ]).unwrap();
    write_wide_part(&dim2, &schema, &[
        vec![Some("7".into())], vec![Some("7".into())], vec![Some("7".into())],
    ]).unwrap();
    let paths = [fact, dim1, dim2];
    let query = parse_join(
        "SELECT count(*) FROM fact JOIN dim1 ON fact.id = dim1.id JOIN dim2 ON dim1.id = dim2.id"
    ).unwrap().unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &paths, &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "count\n420000\n");

    let query = parse_join(
        "SELECT count(*) FROM fact ANY JOIN dim1 ON fact.id = dim1.id ANY JOIN dim2 ON dim1.id = dim2.id"
    ).unwrap().unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &paths, &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "count\n70000\n");
    fs::remove_dir_all(root).unwrap();
}
