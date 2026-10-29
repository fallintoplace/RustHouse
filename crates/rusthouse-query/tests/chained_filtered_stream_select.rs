use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn filtered_page_offsets_count_only_surviving_rows_over_large_fact_scan() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-streamed-join-filter-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let fact = root.join("fact");
    let dim = root.join("dim");
    let tags = root.join("tags");
    let ids = [Column { name:"id".into(), data_type:"UInt64".into() }];
    let facts = (0..70_000).map(|_| vec![Some("7".into())]).collect::<Vec<_>>();
    write_wide_part(&fact, &ids, &facts).unwrap();
    write_wide_part(&dim, &ids, &[vec![Some("7".into())]]).unwrap();
    write_wide_part(&tags, &[
        ids[0].clone(),
        Column { name:"label".into(), data_type:"Nullable(String)".into() },
    ], &[
        vec![Some("7".into()), None],
        vec![Some("7".into()), Some("matched".into())],
    ]).unwrap();
    let paths = [fact, dim, tags];
    let sql = "SELECT fact.id, tags.label FROM fact JOIN dim ON fact.id = dim.id JOIN tags ON dim.id = tags.id WHERE tags.label IS NOT NULL LIMIT 2 OFFSET 3";
    let mut out = Vec::new();
    execute_multi_join(&parse_join(sql).unwrap().unwrap(), &paths, &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "fact.id\ttags.label\n7\tmatched\n7\tmatched\n");
    fs::remove_dir_all(root).unwrap();
}
