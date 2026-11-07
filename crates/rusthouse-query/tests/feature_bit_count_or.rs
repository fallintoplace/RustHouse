use rusthouse_query::{Projection, parse_select};

#[test]
fn supports_bit_count_or() {
    for (sql, expected) in [
        ("SELECT bitCountOr(8,3)", 3),
        ("SELECT bitCountOr(-1,0)", 64),
        ("SELECT bitCountOr(0,0)", 0),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn rejects_invalid_bit_count_or_arguments() {
    for sql in ["SELECT bitCountOr(1)", "SELECT bitCountOr(1,2,3)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
