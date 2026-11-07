use rusthouse_query::{Projection, parse_select};

#[test]
fn supports_bit_count_and_not() {
    for (sql, expected) in [
        ("SELECT bitCountAndNot(15,3)", 2),
        ("SELECT bitCountAndNot(8,15)", 0),
        ("SELECT bitCountAndNot(-1,-1)", 0),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn rejects_invalid_bit_count_and_not_arguments() {
    for sql in ["SELECT bitCountAndNot(1)", "SELECT bitCountAndNot(1,2,3)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
