use rusthouse_query::{execute_multi_join, parse_join};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn chained_min_max_support_text_ignore_nulls_and_keep_empty_strings() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-multi-join-string-extrema-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let fact = root.join("fact");
    let dimension = root.join("dimension");
    write_wide_part(
        &fact,
        &[
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "note".into(),
                data_type: "Nullable(String)".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("zeta".into())],
            vec![Some("2".into()), None],
            vec![Some("3".into()), Some("alpha".into())],
            vec![Some("4".into()), Some("beta".into())],
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
                name: "label".into(),
                data_type: "String".into(),
            },
        ],
        &[
            vec![Some("1".into()), Some("gamma".into())],
            vec![Some("2".into()), Some("zeta".into())],
            vec![Some("3".into()), Some("alpha".into())],
            vec![Some("4".into()), Some(String::new())],
        ],
    )
    .unwrap();

    let query = parse_join(
        "SELECT MIN(f.note) AS min_note, MAX(f.note) AS max_note, MIN(d.label) AS min_label, MAX(d.label) AS max_label, MIN(f.id) AS min_id, MAX(f.id) AS max_id FROM fact f JOIN dimension d ON f.id = d.id",
    )
    .unwrap()
    .unwrap();
    let mut output = Vec::new();
    execute_multi_join(&query, &[fact.clone(), dimension.clone()], &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "min_note\tmax_note\tmin_label\tmax_label\tmin_id\tmax_id\nalpha\tzeta\t\tzeta\t1\t4\n"
    );

    let single_query = parse_join(
        "SELECT MIN(f.note) AS min_note FROM fact f JOIN dimension d ON f.id = d.id JOIN dimension d2 ON d.id = d2.id",
    )
    .unwrap()
    .unwrap();
    let mut single_output = Vec::new();
    execute_multi_join(
        &single_query,
        &[fact.clone(), dimension.clone(), dimension.clone()],
        &mut single_output,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(single_output).unwrap(),
        "min_note\nalpha\n"
    );

    let empty_query = parse_join(
        "SELECT MIN(f.note) AS min_note, MAX(d.label) AS max_label FROM fact f JOIN dimension d ON f.id = d.id WHERE d.label = 'missing'",
    )
    .unwrap()
    .unwrap();
    let mut empty_output = Vec::new();
    execute_multi_join(
        &empty_query,
        &[fact.clone(), dimension.clone()],
        &mut empty_output,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(empty_output).unwrap(),
        "min_note\tmax_label\n\\N\t\\N\n"
    );

    fs::remove_dir_all(root).unwrap();
}
