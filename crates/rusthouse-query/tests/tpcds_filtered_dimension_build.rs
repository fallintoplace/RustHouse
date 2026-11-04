use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn filtered_dimension_build_matches_join_where_semantics() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-filtered-dimension-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let dimension = root.join("dimension");
    let calendar = root.join("calendar");
    let key = [Column {
        name: "id".into(),
        data_type: "UInt64".into(),
    }];
    write_wide_part(
        &fact,
        &key,
        &[
            vec![Some("1".into())],
            vec![Some("2".into())],
            vec![Some("3".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &calendar,
        &key,
        &[
            vec![Some("1".into())],
            vec![Some("2".into())],
            vec![Some("3".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &dimension,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "active".into(),
                data_type: "Nullable(UInt8)".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("1".into())],
            vec![Some("1".into()), Some("0".into())],
            vec![Some("2".into()), None],
            vec![Some("3".into()), Some("1".into())],
        ],
    )
    .unwrap();
    let paths = [fact, dimension, calendar];
    let prefix =
        "SELECT f.id FROM fact f JOIN dimension d ON f.id = d.id JOIN calendar c ON d.id = c.id";
    let run = |sql: &str| {
        let mut out = Vec::new();
        let query = parse_join(sql).unwrap().unwrap();
        execute_multi_join(&query, &paths, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    };
    assert_eq!(run(&format!("{prefix} WHERE d.active = 1")), "f.id\n1\n3\n");
    assert_eq!(run(&format!("{prefix} WHERE d.active = 0")), "f.id\n1\n");
    assert_eq!(run(&format!("{prefix} WHERE d.active = 5")), "f.id\n");
    fs::remove_dir_all(root).unwrap();
}
