use rusthouse_query::{execute, parse_scalar_expression, parse_select, ScalarExpr};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn comparison_binds_before_and_and_before_or() {
    let expression = parse_scalar_expression(
        "value > 10 OR value >= 5 AND enabled = 1"
    ).unwrap();
    let ScalarExpr::Or(_, right) = expression else {
        panic!("expected top-level OR");
    };
    assert!(matches!(*right, ScalarExpr::And(..)));
    for text in ["value > 10 AND", "value = 1 OR", "value < 1 AND OR value > 2"] {
        assert!(parse_scalar_expression(text).is_err(), "{text}");
    }
}

#[test]
fn complex_case_conditions_use_correct_boolean_precedence_with_nulls() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-case-boolean-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let cols = [
        Column {name:"value".into(), data_type:"Nullable(Int64)".into()},
        Column {name:"enabled".into(), data_type:"UInt8".into()},
    ];
    let rows = [
        vec![Some("12".into()), Some("0".into())],
        vec![Some("6".into()), Some("1".into())],
        vec![Some("6".into()), Some("0".into())],
        vec![None, Some("0".into())],
    ];
    write_wide_part(&root, &cols, &rows).unwrap();
    let query = parse_select(
        "SELECT sum(CASE WHEN value > 10 OR value >= 5 AND enabled = 1 THEN 2 ELSE 0 END) FROM part"
    ).unwrap();
    let mut output = Vec::new();
    execute(&query, Some(&root), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap().lines().last(), Some("4"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn short_circuiting_avoids_dividing_by_zero_in_unvisited_boolean_branch() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-case-short-circuit-{}-{}",
        std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let cols = [Column {name:"value".into(), data_type:"UInt8".into()}];
    let rows = [
        vec![Some("0".into())], vec![Some("3".into())],
    ];
    write_wide_part(&root, &cols, &rows).unwrap();
    let query = parse_select(
        "SELECT sum(CASE WHEN value = 0 OR 10 / value > 2 THEN 1 ELSE 0 END), sum(CASE WHEN value != 0 AND 10 / value > 2 THEN 1 ELSE 0 END) FROM part"
    ).unwrap();
    let mut output = Vec::new();
    execute(&query, Some(&root), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap().lines().last(), Some("2\t1"));
    fs::remove_dir_all(root).unwrap();
}
