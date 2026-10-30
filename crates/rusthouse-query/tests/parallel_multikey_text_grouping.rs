use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn groups(root: &Path, sql: &str) -> Vec<String> {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(root), &mut output).unwrap();
    let output = String::from_utf8(output).unwrap();
    let mut lines = output.lines();
    assert_eq!(lines.next(), Some("region\tsegment\tcount"));
    let mut result = lines.map(str::to_owned).collect::<Vec<_>>();
    result.sort_unstable();
    result
}

#[test]
fn parallel_text_groups_preserve_null_empty_utf8_and_tuple_boundaries() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-parallel-text-group-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let columns = [
        Column {
            name: "region".into(),
            data_type: "Nullable(String)".into(),
        },
        Column {
            name: "segment".into(),
            data_type: "String".into(),
        },
        Column {
            name: "active".into(),
            data_type: "UInt8".into(),
        },
    ];
    let rows: Vec<Vec<Option<String>>> = vec![
        vec![Some("a".into()), Some("bc".into()), Some("1".into())],
        vec![Some("ab".into()), Some("c".into()), Some("0".into())],
        vec![None, Some("bc".into()), Some("1".into())],
        vec![Some("".into()), Some("bc".into()), Some("1".into())],
        vec![Some("é".into()), Some("ß".into()), Some("1".into())],
        vec![Some("a".into()), Some("bc".into()), Some("0".into())],
    ];
    // More than 16 parts selects the parallel multi-key scan on multicore hosts.
    for part in 1..=24 {
        write_wide_part(
            &root.join(format!("all_{part}_{part}_0")),
            &columns,
            &rows,
        )
        .unwrap();
    }

    let mut all = vec![
        "\tbc\t24",
        "NULL\tbc\t24",
        "a\tbc\t48",
        "ab\tc\t24",
        "é\tß\t24",
    ];
    all.sort_unstable();
    assert_eq!(
        groups(
            &root,
            "SELECT region, segment, count(*) FROM part GROUP BY region, segment",
        ),
        all
    );

    let mut filtered = vec![
        "\tbc\t24",
        "NULL\tbc\t24",
        "a\tbc\t24",
        "é\tß\t24",
    ];
    filtered.sort_unstable();
    assert_eq!(
        groups(
            &root,
            "SELECT region, segment, count(*) FROM part WHERE active = 1 GROUP BY region, segment",
        ),
        filtered
    );

    fs::remove_dir_all(root).unwrap();
}
