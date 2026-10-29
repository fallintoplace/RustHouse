use rusthouse_query::{execute, parse_scalar_expression, parse_select, ScalarExpr};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn case_when_expression_preserves_column_references_and_precedence() {
    let parsed = parse_scalar_expression(
        "CASE WHEN value * 2 > threshold THEN value + 1 ELSE threshold - 1 END"
    ).unwrap();
    let mut columns = Vec::new();
    parsed.referenced_columns(&mut columns);
    assert_eq!(columns, ["value", "threshold"]);
    assert!(matches!(parsed, ScalarExpr::CaseWhen(..)));
    assert!(parse_scalar_expression("CASE WHEN a > 1 THEN a END").is_err());
    assert!(parse_scalar_expression("CASE WHEN a > 1 ELSE a END").is_err());
}

#[test]
fn conditional_numeric_aggregates_choose_only_matching_case_branch() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-case-aggregate-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column {name:"value".into(), data_type:"Int64".into()},
        Column {name:"threshold".into(), data_type:"Int64".into()},
    ];
    let rows = [
        vec![Some("5".into()), Some("10".into())],
        vec![Some("11".into()), Some("10".into())],
        vec![Some("20".into()), Some("10".into())],
        vec![None, Some("10".into())],
    ];
    // Nullable value tests SQL NULL conditions, with the ELSE path selected.
    let columns = [
        Column {name:"value".into(), data_type:"Nullable(Int64)".into()},
        columns[1].clone(),
    ];
    write_wide_part(&root, &columns, &rows).unwrap();
    let query = parse_select(
        "SELECT sum(CASE WHEN value > threshold THEN value ELSE 0 END), count(*) FROM part"
    ).unwrap();
    let mut out = Vec::new();
    execute(&query, Some(&root), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.lines().last().unwrap().ends_with("31\t4"), "{text}");
    fs::remove_dir_all(root).unwrap();
}
