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

#[test]
fn nvl_matches_ifnull() {
    assert_eq!(evaluate("nvl(x, y)"), "10");
}

#[test]
fn isnull_counts_missing_numeric_values() {
    assert_eq!(evaluate("isNull(x)"), "1");
}

#[test]
fn isnotnull_counts_present_numeric_values() {
    assert_eq!(evaluate("isNotNull(x)"), "3");
}

#[test]
fn plus_preserves_signed_numeric_arithmetic() {
    assert_eq!(evaluate("plus(x, y)"), "5");
}

#[test]
fn minus_preserves_signed_numeric_arithmetic() {
    assert_eq!(evaluate("minus(x, y)"), "1");
}

#[test]
fn multiply_handles_negative_numeric_inputs() {
    assert_eq!(evaluate("multiply(x, y)"), "-11");
}

#[test]
fn divide_uses_infix_numeric_semantics() {
    assert_eq!(evaluate("divide(x, if(y = 0, 1, y))"), evaluate("x / if(y = 0, 1, y)"));
}

#[test]
fn modulo_preserves_signed_remainder() {
    assert_eq!(evaluate("modulo(x, if(y = 0, 1, y))"), "-2");
}

#[test]
fn negate_propagates_null_and_signed_values() {
    assert_eq!(evaluate("negate(x)"), "-3");
}

#[test]
fn positive_preserves_nullable_numeric_values() {
    assert_eq!(evaluate("positive(x)"), "3");
}

#[test]
fn equals_returns_nullable_integer_truth() {
    assert_eq!(evaluate("equals(x, y)"), "1");
}

#[test]
fn not_equals_preserves_three_valued_logic() {
    assert_eq!(evaluate("notEquals(x, y)"), "2");
}

#[test]
fn less_uses_exact_comparison_semantics() {
    assert_eq!(evaluate("less(x, y)"), "1");
}

#[test]
fn less_or_equals_includes_equal_rows() {
    assert_eq!(evaluate("lessOrEquals(x, y)"), "2");
}

#[test]
fn greater_compares_signed_numeric_inputs() {
    assert_eq!(evaluate("greater(x, y)"), "1");
}

#[test]
fn greater_or_equals_includes_equal_rows() {
    assert_eq!(evaluate("greaterOrEquals(x, y)"), "2");
}
