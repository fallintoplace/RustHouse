use rusthouse_query::{parse_select, Projection};

#[test]
fn clickhouse_positive_modulo_scalar_parity() {
    for (expression, expected) in [
        ("positiveModulo(-3, 10)", 7),
        ("positiveModulo(3, 10)", 3),
        ("positiveModulo(-10, 5)", 0),
    ] {
        let sql = format!("SELECT {expression}");
        assert_eq!(parse_select(&sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
    for expression in ["positiveModulo(1)", "positiveModulo(1, 0)", "positiveModulo(1, -2)"] {
        assert!(parse_select(&format!("SELECT {expression}")).is_err(), "{expression}");
    }
}
