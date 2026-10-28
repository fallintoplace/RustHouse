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
        ("SELECT multiply(6, 7)", 42),
        ("SELECT multiply(-4, 3)", -12),
        ("SELECT multiply(2, 3 + 4)", 14),
    ] {
        assert_eq!(number(sql), expected, "{sql}");
    }
}

#[test]
fn rejects_wrong_arity_and_overflow() {
    for sql in [
        "SELECT multiply()",
        "SELECT multiply(3)",
        "SELECT multiply(3037000500, 3037000500)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
