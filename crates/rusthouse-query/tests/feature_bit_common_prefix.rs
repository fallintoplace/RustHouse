use rusthouse_query::{Projection, parse_select};

#[test]
fn supports_bit_common_prefix() {
    for (sql, expected) in [
        ("SELECT bitCommonPrefix(0,0)", 64),
        ("SELECT bitCommonPrefix(0,1)", 63),
        ("SELECT bitCommonPrefix(-1,0)", 0),
        ("SELECT bitCommonPrefix(8,12)", 61),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn rejects_invalid_bit_common_prefix_arguments() {
    for sql in ["SELECT bitCommonPrefix(1)", "SELECT bitCommonPrefix(1,2,3)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
