use rusthouse_query::{execute, parse_select};

#[test]
fn numbers_accepts_more_than_a_million_rows_without_materializing_them() {
    for (sql, expected) in [
        ("SELECT count(*) FROM numbers(8000000)", "count\n8000000\n"),
        ("SELECT count(*) FROM numbers(10, 8000000)", "count\n8000000\n"),
        ("SELECT count(*) FROM numbers(10, 8000000, 2)", "count\n4000000\n"),
    ] {
        let query = parse_select(sql).unwrap();
        let mut output = Vec::new();
        execute(&query, None, &mut output).unwrap();
        assert_eq!(output, expected.as_bytes());
    }
    assert!(parse_select("SELECT number FROM numbers(10000001)").is_err());
    assert!(parse_select("SELECT number FROM numbers(1, 10000001)").is_err());
}
