use rusthouse_query::{execute, parse_select};

#[test]
fn numbers_accepts_more_than_a_million_rows_without_materializing_them() {
    for (sql, expected) in [
        ("SELECT count(*) FROM numbers(8000000)", "count\n8000000\n"),
        (
            "SELECT count(*) FROM numbers(10, 8000000)",
            "count\n8000000\n",
        ),
        (
            "SELECT count(*) FROM numbers(10, 8000000, 2)",
            "count\n4000000\n",
        ),
    ] {
        let query = parse_select(sql).unwrap();
        let mut output = Vec::new();
        execute(&query, None, &mut output).unwrap();
        assert_eq!(output, expected.as_bytes());
    }
    assert!(parse_select("SELECT number FROM numbers(1000000001)").is_err());
    assert!(parse_select("SELECT number FROM numbers(1, 1000000001)").is_err());
}

#[test]
fn billion_row_virtual_count_requires_arithmetic_fast_path() {
    for (sql, expected) in [
        (
            "SELECT count(*) FROM numbers(1000000000)",
            "count\n1000000000\n",
        ),
        (
            "SELECT count(number) FROM numbers(1000000000) WHERE number >= 999999995",
            "count(number)\n5\n",
        ),
        (
            "SELECT count(DISTINCT number) FROM numbers(1000000000) WHERE number < 5",
            "countDistinct(number)\n5\n",
        ),
    ] {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
        assert_eq!(output, expected.as_bytes(), "{sql}");
    }
    for (sql, expected) in [
        ("SELECT number FROM numbers(1000000000) LIMIT 1", "number\n0\n"),
        (
            "SELECT sum(number) FROM numbers(1000000000)",
            "sum(number)\n499999999500000000\n",
        ),
    ] {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
        assert_eq!(output, expected.as_bytes(), "{sql}");
    }
    for sql in [
        "SELECT number FROM numbers(1000000000)",
        "SELECT count(*) FROM numbers(1000000000) WHERE number % 7 = 0 AND number > 10",
    ] {
        let mut output = Vec::new();
        assert!(
            execute(&parse_select(sql).unwrap(), None, &mut output).is_err(),
            "{sql}"
        );
    }
}
