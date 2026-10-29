use rusthouse_query::{execute, parse_scalar_expression, parse_select, ScalarExpr};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn searched_case_supports_many_when_arms_and_implicit_null() {
    let case = parse_scalar_expression(
        "CASE WHEN score >= 10 THEN 2 WHEN score >= 5 THEN 1 END"
    ).unwrap();
    let mut names = Vec::new();
    case.referenced_columns(&mut names);
    assert_eq!(names, ["score"]);
    assert!(matches!(case, ScalarExpr::CaseWhen(..)));
    let many = format!("CASE {} END", (0..33)
        .map(|n| format!("WHEN score = {n} THEN {n}"))
        .collect::<Vec<_>>().join(" "));
    assert!(parse_scalar_expression(&many).is_err());
}

#[test]
fn multi_when_aggregates_follow_first_match_and_null_default() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-multi-case-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [Column {name:"score".into(), data_type:"Nullable(Int64)".into()}];
    let rows = [
        vec![Some("2".into())], vec![Some("7".into())],
        vec![Some("12".into())], vec![None],
    ];
    write_wide_part(&root, &columns, &rows).unwrap();
    let query = parse_select(
        "SELECT sum(CASE WHEN score >= 10 THEN 2 WHEN score >= 5 THEN 1 END), count(CASE WHEN score >= 10 THEN 1 WHEN score >= 5 THEN 1 END) FROM part"
    ).unwrap();
    let mut out = Vec::new();
    execute(&query, Some(&root), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.lines().last(), Some("3\t2"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn nested_case_only_evaluates_chosen_branch() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-nested-case-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [Column {name:"score".into(), data_type:"UInt8".into()}];
    let rows = [
        vec![Some("12".into())], vec![Some("1".into())],
    ];
    write_wide_part(&root, &columns, &rows).unwrap();
    let query = parse_select(
        "SELECT sum(CASE WHEN score > 10 THEN CASE WHEN score > 15 THEN 1 ELSE 2 END ELSE 3 END) FROM part"
    ).unwrap();
    let mut out = Vec::new();
    execute(&query, Some(&root), &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap().lines().last(), Some("5"));
    fs::remove_dir_all(root).unwrap();
}
