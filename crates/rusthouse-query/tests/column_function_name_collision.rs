use rusthouse_query::{parse_select, Projection};

#[test]
fn function_names_without_parentheses_are_column_identifiers() {
    for column in ["positive", "minus", "abs", "upper", "lower", "power", "iszero"] {
        let sql = format!("SELECT {column} FROM data");
        let query = parse_select(&sql).unwrap();
        match query.projection {
            Projection::Columns(columns) => assert_eq!(columns, vec![column], "{sql}"),
            projection => panic!("expected a column for {sql}, got {projection:?}"),
        }
    }
}

#[test]
fn function_named_group_key_keeps_grouped_count() {
    let query = parse_select(
        "SELECT positive, count(*) FROM events GROUP BY positive ORDER BY positive",
    )
    .unwrap();
    assert!(!matches!(query.projection, Projection::Constants(_) | Projection::Literal(_)));
}

#[test]
fn parenthesized_scalar_functions_still_parse_as_constants() {
    for sql in ["SELECT positive(9)", "SELECT minus(10, 3)", "SELECT upper('abc')"] {
        let query = parse_select(sql).unwrap();
        assert!(
            matches!(query.projection, Projection::Constants(_) | Projection::Literal(_)),
            "{sql}"
        );
    }
}
