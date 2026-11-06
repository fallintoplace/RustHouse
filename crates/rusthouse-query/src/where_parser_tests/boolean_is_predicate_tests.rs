use super::*;
use crate::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn boolean_is_forms_have_stable_numeric_truthiness() {
    assert_eq!(
        run("SELECT number FROM numbers(4) WHERE number IS TRUE ORDER BY number"),
        "number\n1\n2\n3\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(4) WHERE number IS FALSE"),
        "number\n0\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(4) WHERE number IS NOT TRUE"),
        "number\n0\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(4) WHERE number IS NOT FALSE ORDER BY number"),
        "number\n1\n2\n3\n"
    );
}

#[test]
fn negated_is_predicates_preserve_sql_null_alternative() {
    let query = parse_select("SELECT number FROM numbers(3) WHERE number IS NOT TRUE").unwrap();
    assert_eq!(query.or_predicates.len(), 1);
    assert!(matches!(
        query.or_predicates[0].as_slice(),
        [Predicate::IsNull { negated: false, .. }]
    ));
    assert_eq!(
        run("SELECT number FROM numbers(3) WHERE number IS NOT UNKNOWN ORDER BY number"),
        "number\n0\n1\n2\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(3) WHERE number IS UNKNOWN"),
        "number\n"
    );
    assert!(parse_select("SELECT number FROM numbers(3) WHERE number IS MAYBE").is_err());
}
