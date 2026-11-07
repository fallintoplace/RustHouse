use rusthouse_query::{Projection, parse_select};

#[test]
fn supports_bit_common_suffix() {
    for (sql, expected) in [
        ("SELECT bitCommonSuffix(0,0)", 64),
        ("SELECT bitCommonSuffix(2,6)", 2),
        ("SELECT bitCommonSuffix(1,0)", 0),
        ("SELECT bitCommonSuffix(-1,-1)", 64),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn rejects_invalid_bit_common_suffix_arguments() {
    for sql in ["SELECT bitCommonSuffix(1)", "SELECT bitCommonSuffix(1,2,3)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
