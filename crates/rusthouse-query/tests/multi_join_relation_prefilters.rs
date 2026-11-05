use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn shared_filters_on_multiple_relations_preserve_or_branch_results() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-multi-join-relation-prefilters-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let labels = root.join("labels");
    let flags = root.join("flags");

    write_wide_part(
        &fact,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "amount".into(),
                data_type: "UInt64".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("5".into())],
            vec![Some("1".into()), Some("7".into())],
            vec![Some("2".into()), Some("11".into())],
            vec![Some("3".into()), Some("13".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &labels,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "keep".into(),
                data_type: "String".into(),
            },
            Column {
                name: "variant".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("yes".into()), Some("x".into())],
            vec![Some("1".into()), Some("yes".into()), Some("y".into())],
            vec![Some("2".into()), Some("no".into()), Some("x".into())],
            vec![Some("3".into()), Some("yes".into()), Some("z".into())],
        ],
    )
    .unwrap();
    write_wide_part(
        &flags,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "keep".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("yes".into())],
            vec![Some("2".into()), Some("yes".into())],
            vec![Some("3".into()), Some("no".into())],
        ],
    )
    .unwrap();

    let query = parse_join(
        "SELECT SUM(f.amount) AS total FROM fact f JOIN labels l ON f.id = l.id JOIN flags g ON f.id = g.id WHERE f.amount > 5 AND l.keep = 'yes' AND g.keep = 'yes' AND (l.variant = 'x' OR l.variant = 'y')",
    )
    .unwrap()
    .unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &[fact, labels, flags], &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "total\n14\n");

    fs::remove_dir_all(root).unwrap();
}
