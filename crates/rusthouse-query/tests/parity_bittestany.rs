use rusthouse_query::{parse_select, Projection};

#[test]
fn clickhouse_bittestany_scalar_parity() {
    for (expression, expected) in [
        ("bitTestAny(5, 3)", 1),
        ("bitTestAny(4, 3)", 0),
        ("bitTestAny(0, 0)", 0),
        ("bitTestAny(-1, 8)", 1),
    ] {
        let sql = format!("SELECT {expression}");
        assert_eq!(parse_select(&sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
    for expression in ["bitTestAny(1)", "bitTestAny(1, 2, 3)"] {
        assert!(parse_select(&format!("SELECT {expression}")).is_err(), "{expression}");
    }
}
