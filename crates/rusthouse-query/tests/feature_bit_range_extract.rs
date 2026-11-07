use rusthouse_query::{Projection, parse_select};

#[test]
fn supports_bit_range_extract() {
    for (sql, expected) in [
        ("SELECT bitRangeExtract(240,4,4)", 15),
        ("SELECT bitRangeExtract(255,1,3)", 7),
        ("SELECT bitRangeExtract(10,0,0)", 0),
        ("SELECT bitRangeExtract(-1,60,4)", 15),
        ("SELECT bitRangeExtract(-1,0,64)", -1),
        ("SELECT bitRangeExtract(1,64,0)", 0),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn rejects_invalid_bit_range_extract_arguments() {
    for sql in ["SELECT bitRangeExtract(1,2)", "SELECT bitRangeExtract(1,2,3,4)", "SELECT bitRangeExtract(1,-1,1)", "SELECT bitRangeExtract(1,63,2)", "SELECT bitRangeExtract(1,0,-1)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
