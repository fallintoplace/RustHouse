use rusthouse_query::{Projection, parse_select};

#[test]
fn supports_bit_rank() {
    for (sql, expected) in [
        ("SELECT bitRank(45,0)", 0),
        ("SELECT bitRank(45,4)", 3),
        ("SELECT bitRank(-1,64)", 64),
        ("SELECT bitRank(0,64)", 0),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn rejects_invalid_bit_rank_arguments() {
    for sql in ["SELECT bitRank(1)", "SELECT bitRank(1,2,3)", "SELECT bitRank(1,-1)", "SELECT bitRank(1,65)"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
