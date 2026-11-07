use rusthouse_query::{Projection, parse_select};

#[test]
fn supports_bit_count_and() {
    for (sql, expected) in [
        ("SELECT bitCountAnd(15,10)", 2),
        ("SELECT bitCountAnd(-1,5)", 2),
        ("SELECT bitCountAnd(0,0)", 0),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn rejects_invalid_bit_count_and_arguments() {
    for sql in ["SELECT bitCountAnd(1)", "SELECT bitCountAnd(1,2,3)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
