use rusthouse_query::{split_clickhouse_format, parse_select};

#[test]
fn extracts_terminal_format_after_a_select() {
    for (sql, expected_sql, expected_format) in [
        ("SELECT 1 FORMAT CSV", "SELECT 1", "CSV"),
        ("SELECT number FROM numbers(3) FORMAT TabSeparated;", "SELECT number FROM numbers(3)", "TabSeparated"),
        ("SELECT 'FORMAT CSV' AS label FORMAT JSONStringsEachRow", "SELECT 'FORMAT CSV' AS label", "JSONStringsEachRow"),
    ] {
        let (statement, name) = split_clickhouse_format(sql).unwrap();
        assert_eq!((statement, name), (expected_sql, Some(expected_format)));
        assert!(parse_select(statement).is_ok());
    }
}

#[test]
fn does_not_interpret_quoted_text_or_comments_as_format() {
    for sql in [
        "SELECT 'FORMAT CSV'",
        "SELECT 1 /* FORMAT CSV */",
        "SELECT 1 -- FORMAT CSV",
        "SELECT 1",
    ] {
        assert!(split_clickhouse_format(sql).unwrap().1.is_none(), "{sql}");
    }
}
