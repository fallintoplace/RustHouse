use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

// This fixture covers negative, zero, positive, and NULL numeric inputs.
fn evaluate(expression: &str) -> String {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-agg-builtin-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    let cols = [
        Column { name: "x".into(), data_type: "Nullable(Int64)".into() },
        Column { name: "y".into(), data_type: "Int64".into() },
    ];
    let rows = [
        vec![Some("-2".into()), Some("3".into())],
        vec![Some("0".into()), Some("0".into())],
        vec![Some("5".into()), Some("-1".into())],
        vec![None, Some("7".into())],
    ];
    write_wide_part(&root, &cols, &rows).unwrap();
    let query = parse_select(&format!("SELECT sum({expression}) FROM part")).unwrap();
    let mut output = Vec::new();
    execute(&query, Some(&root), &mut output).unwrap();
    let result = String::from_utf8(output).unwrap();
    fs::remove_dir_all(root).unwrap();
    result.lines().last().unwrap().to_owned()
}

#[test]
fn ifnull_substitutes_null_aggregate_inputs() {
    assert_eq!(evaluate("ifNull(x, y)"), "10");
}
