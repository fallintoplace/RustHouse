use rusthouse_query::{execute, parse_select};

#[test]
fn numbers_accepts_more_than_a_million_rows_without_materializing_them() {
    for sql in [
        "SELECT count(*) FROM numbers(8000000)",
        "SELECT count(*) FROM numbers(10, 8000000)",
        "SELECT count(*) FROM numbers(10, 8000000, 2)",
    ] {
        let query = parse_select(sql).unwrap();
        let mut output = Vec::new();
        execute(&query, None, &mut output).unwrap();
        assert_eq!(output, b"count\n8000000\n");
    }
    assert!(parse_select("SELECT number FROM numbers(10000001)").is_err());
    assert!(parse_select("SELECT number FROM numbers(1, 10000001)").is_err());
}
