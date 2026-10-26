use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn fixture(parts: &[&[u64]]) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-offset-skip-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    for (part, values) in parts.iter().enumerate() {
        let rows = values
            .iter()
            .map(|value| vec![Some(value.to_string())])
            .collect::<Vec<_>>();
        write_wide_part(
            &root.join(format!("all_{part}_{part}_0")),
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &rows,
        )
        .unwrap();
    }
    root
}

fn run(root: &Path, sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, Some(root), &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn offset_crosses_granules_and_parts_without_dropping_partial_marks() {
    // The first part has three adaptive marks (1024, 1024, 2).
    let first = (0..2050).collect::<Vec<u64>>();
    let second = (2050..2057).collect::<Vec<u64>>();
    let root = fixture(&[&first, &second]);

    assert_eq!(
        run(&root, "SELECT id FROM part LIMIT 4 OFFSET 1022"),
        "id\n1022\n1023\n1024\n1025\n"
    );
    assert_eq!(
        run(&root, "SELECT id FROM part LIMIT 4 OFFSET 2048"),
        "id\n2048\n2049\n2050\n2051\n"
    );
    assert_eq!(
        run(&root, "SELECT id FROM part LIMIT 4 OFFSET 2050"),
        "id\n2050\n2051\n2052\n2053\n"
    );
    assert_eq!(
        run(&root, "SELECT id FROM part LIMIT 3 OFFSET 2057"),
        "id\n"
    );
    assert_eq!(
        run(&root, "SELECT id FROM part LIMIT 3 OFFSET 999999"),
        "id\n"
    );

    // Filtering and sorting must not use physical scan offsets.
    assert_eq!(
        run(
            &root,
            "SELECT id FROM part WHERE id >= 2052 LIMIT 2 OFFSET 1"
        ),
        "id\n2053\n2054\n"
    );
    assert_eq!(
        run(
            &root,
            "SELECT id FROM part ORDER BY id DESC LIMIT 2 OFFSET 1"
        ),
        "id\n2055\n2054\n"
    );
    assert_eq!(run(&root, "SELECT count(*) FROM part"), "count\n2057\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn distinct_offsets_count_unique_rows_not_storage_rows() {
    let root = fixture(&[&[1, 1, 2], &[2, 3, 3, 4]]);
    assert_eq!(
        run(&root, "SELECT DISTINCT id FROM part LIMIT 2 OFFSET 1"),
        "id\n2\n3\n"
    );
    fs::remove_dir_all(root).unwrap();
}
