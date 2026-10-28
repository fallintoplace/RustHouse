use rusthouse_query::{Projection, parse_select};

fn number(sql: &str) -> i64 {
    match parse_select(sql).unwrap().projection {
        Projection::Literal(value) => value,
        other => panic!("unexpected projection: {other:?}"),
    }
}

#[test]
fn computes_checked_int64_results() {
    for (sql, expected) in [
        ("SELECT plus(1, 2)", 3),
        ("SELECT plus(-3, 1)", -2),
        ("SELECT plus(2, 3 * 4)", 14),
    ] {
        assert_eq!(number(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_wrong_arity_and_overflow() {
    for sql in [
        "SELECT plus()",
        "SELECT plus(1)",
        "SELECT plus(9223372036854775807, 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
