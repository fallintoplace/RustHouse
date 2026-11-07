use rusthouse_query::{Projection, parse_select};

#[test]
fn supports_bit_select() {
    for (sql, expected) in [
        ("SELECT bitSelect(45,0)", 0),
        ("SELECT bitSelect(45,1)", 2),
        ("SELECT bitSelect(45,3)", 5),
        ("SELECT bitSelect(45,4)", -1),
        ("SELECT bitSelect(-1,63)", 63),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn rejects_invalid_bit_select_arguments() {
    for sql in ["SELECT bitSelect(1)", "SELECT bitSelect(1,2,3)", "SELECT bitSelect(1,-1)", "SELECT bitSelect(1,64)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
