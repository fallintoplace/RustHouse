use rusthouse_query::{execute, parse_scalar_expression, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn case_when_string_comparison_with_and_or_aggregates() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-case-text-{}-{}", std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column { name: "priority".into(), data_type: "Nullable(String)".into() },
        Column { name: "score".into(), data_type: "Int64".into() },
    ];
    let rows = [
        vec![Some("urgent".into()), Some("0".into())],
        vec![Some("high".into()), Some("1".into())],
        vec![Some("high".into()), Some("2".into())],
        vec![Some("low".into()), Some("2".into())],
        vec![None, Some("2".into())],
    ];
    write_wide_part(&root, &columns, &rows).unwrap();
    let query = "SELECT sum(CASE WHEN priority = 'urgent' OR priority = 'high' AND score >= 2 THEN 1 ELSE 0 END), sum(CASE WHEN priority = 'low' AND score >= 2 THEN 1 ELSE 0 END) FROM part";
    let mut output = Vec::new();
    execute(&parse_select(query).unwrap(), Some(&root), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap().lines().last(), Some("2\t1"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn text_case_expression_rejects_unbounded_or_untyped_conditions() {
    assert!(parse_scalar_expression("CASE WHEN a = 'abc' THEN 1 ELSE 0 END").is_ok());
    assert!(parse_scalar_expression("CASE WHEN a + 1 = 'abc' THEN 1 ELSE 0 END").is_err());
    assert!(parse_scalar_expression("CASE WHEN a = 'abc' AND THEN 1 END").is_err());
}
