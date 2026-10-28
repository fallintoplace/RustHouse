use rusthouse_query::{parse_select, Projection};

#[test]
fn clickhouse_bittestall_scalar_parity() {
    for (expression, expected) in [
        ("bitTestAll(7, 3)", 1),
        ("bitTestAll(5, 3)", 0),
        ("bitTestAll(0, 0)", 1),
        ("bitTestAll(-1, 8)", 1),
    ] {
        let sql = format!("SELECT {expression}");
        assert_eq!(parse_select(&sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
    for expression in ["bitTestAll(1)", "bitTestAll(1, 2, 3)"] {
        assert!(parse_select(&format!("SELECT {expression}")).is_err(), "{expression}");
    }
}
