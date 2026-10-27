use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn counts_utf8_bytes_and_respects_nulls_filters_and_limits() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-length-columns-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column {
            name: "label".into(),
            data_type: "Nullable(String)".into(),
        },
        Column {
            name: "enabled".into(),
            data_type: "UInt8".into(),
        },
    ];
    let rows = [
        vec![Some("é".into()), Some("1".into())],
        vec![None, Some("0".into())],
        vec![Some("hello".into()), Some("1".into())],
    ];
    write_wide_part(&root, &columns, &rows).unwrap();
    let run = |sql: &str| {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&root), &mut output).unwrap();
        String::from_utf8(output).unwrap()
    };
    assert_eq!(run("SELECT LENGTH(label) FROM part"), "length(label)\n2\n\\N\n5\n");
    assert_eq!(
        run("SELECT LENGTH(label) AS bytes FROM part WHERE enabled = 1 LIMIT 1 OFFSET 1"),
        "bytes\n5\n"
    );
    assert_eq!(run("SELECT LENGTH(label) FROM part LIMIT 0"), "length(label)\n");
    assert!(parse_select("SELECT LENGTH(label) FROM part ORDER BY enabled").is_err());
    fs::remove_dir_all(root).unwrap();
}
