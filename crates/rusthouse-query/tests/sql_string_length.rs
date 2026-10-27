use rusthouse_query::{ConstantValue, Projection, execute, parse_select};

#[test]
fn length_counts_literal_bytes_and_propagates_null() {
    for (sql, expected) in [
        ("SELECT LENGTH('abc')", Projection::Literal(3)),
        ("SELECT char_length('')", Projection::Literal(0)),
        ("SELECT LENGTH('a''b')", Projection::Literal(3)),
        ("SELECT LENGTH('é')", Projection::Literal(2)),
        ("SELECT CHAR_LENGTH('é')", Projection::Literal(1)),
        ("SELECT LENGTH('😀')", Projection::Literal(4)),
        ("SELECT CHAR_LENGTH('😀')", Projection::Literal(1)),
        (
            "SELECT LENGTH(NULL)",
            Projection::Constants(vec![rusthouse_query::Constant {
                value: ConstantValue::Null,
                alias: None,
            }]),
        ),
    ] {
        assert_eq!(parse_select(sql).unwrap().projection, expected, "{sql}");
    }
}

#[test]
fn length_supports_aliases_and_mixed_constant_columns() {
    let query =
        parse_select("SELECT LENGTH('abc') AS n, 'ok' AS text, CHAR_LENGTH('x') AS m").unwrap();
    assert_eq!(
        query.projection,
        Projection::Constants(vec![
            rusthouse_query::Constant {
                value: ConstantValue::Integer(3),
                alias: Some("n".into())
            },
            rusthouse_query::Constant {
                value: ConstantValue::Text("ok".into()),
                alias: Some("text".into())
            },
            rusthouse_query::Constant {
                value: ConstantValue::Integer(1),
                alias: Some("m".into())
            },
        ])
    );
    let mut result = Vec::new();
    execute(&query, None, &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(), "n\ttext\tm\n3\tok\t1\n");
}

#[test]
fn length_rejects_wrong_arity_types_and_from_clause() {
    for sql in [
        "SELECT LENGTH()",
        "SELECT LENGTH(1)",
        "SELECT LENGTH('a', 'b')",
        "SELECT LENGTH('a'",
        "SELECT LENGTH('a') FROM numbers(1)",
        "SELECT LENGTH(LENGTH('x'))",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
