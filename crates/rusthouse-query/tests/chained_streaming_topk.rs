use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn chained_join_topk_sorts_entire_large_fact_before_limit_offset() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-chained-topk-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let a = root.join("a");
    let b = root.join("b");
    let c = root.join("c");
    let facts = [
        Column { name: "id".into(), data_type: "UInt64".into() },
        Column { name: "score".into(), data_type: "UInt64".into() },
    ];
    let schema = [Column { name: "id".into(), data_type: "UInt64".into() }];
    let rows = (0..70_000u64).map(|score| {
        vec![Some("7".into()), Some(score.to_string())]
    }).collect::<Vec<_>>();
    write_wide_part(&a, &facts, &rows).unwrap();
    write_wide_part(&b, &schema, &[vec![Some("7".into())]]).unwrap();
    write_wide_part(&c, &schema, &[vec![Some("7".into())]]).unwrap();
    let query = parse_join(
        "SELECT a.score FROM a JOIN b ON a.id = b.id JOIN c ON b.id = c.id ORDER BY a.score DESC LIMIT 4 OFFSET 1"
    ).unwrap().unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &[a, b, c], &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(),
        "a.score\n69998\n69997\n69996\n69995\n");
    fs::remove_dir_all(root).unwrap();
}
