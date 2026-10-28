use rusthouse_query::{parse_select, Projection};

#[test]
fn clickhouse_positive_modulo_constants() {
    for (sql, expected) in [
        ("SELECT positiveModulo(-3, 10)", 7),
        ("SELECT positiveModulo(17, 10)", 7),
        ("SELECT positiveModulo(0, 7)", 0),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, Projection::Literal(expected), "{sql}");
    }
}

#[test]
fn clickhouse_positive_modulo_rejects_invalid_inputs() {
    for sql in [
        "SELECT positiveModulo(-3, 0)",
        "SELECT positiveModulo(-3, -10)",
        "SELECT positiveModulo(1)",
        "SELECT positiveModulo(3, 2, 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
