use rusthouse_query::{parse_select, Projection};

#[test]
fn clickhouse_minus_scalar_parity() {
    for (expression, expected) in [
        ("minus(10, 3)", 7),
        ("minus(3, 10)", -7),
        ("minus(minus(5, 2), 1)", 2),
    ] {
        let sql = format!("SELECT {expression}");
        assert_eq!(parse_select(&sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
    for expression in ["minus(1)", "minus(1, 2, 3)", "minus(-9223372036854775807, 2)"] {
        assert!(parse_select(&format!("SELECT {expression}")).is_err(), "{expression}");
    }
}
