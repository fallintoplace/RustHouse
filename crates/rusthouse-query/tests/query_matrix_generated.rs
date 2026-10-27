use rusthouse_query::{execute, parse_select};

#[test]
fn generated_sources_preserve_filter_and_pagination_results() {
    for (sql, expected) in [
        (
            "SELECT number FROM numbers(20) WHERE number >= 5 AND number % 3 = 2 ORDER BY number DESC LIMIT 3 OFFSET 1",
            "number\n14\n11\n8\n",
        ),
        (
            "SELECT count(*) FROM generate_series(3, 39, 4) WHERE number BETWEEN 11 AND 31",
            "count\n6\n",
        ),
        (
            "SELECT number FROM numbers(5, 15) WHERE number NOT IN (6, 8, 10, 12, 14, 16, 18) ORDER BY number LIMIT 4 OFFSET 2",
            "number\n9\n11\n13\n15\n",
        ),
        (
            "SELECT sum(number), count(*), avg(number) FROM numbers(40) WHERE number % 10 = 0",
            "sum(number)\tcount(*)\tavg(number)\n60\t4\t15\n",
        ),
    ] {
        let parsed = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
        let mut output = Vec::new();
        execute(&parsed, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
        assert_eq!(String::from_utf8(output).unwrap(), expected, "{sql}");
    }
}
