use rusthouse_query::{execute, parse_select};

#[test]
fn generated_boundaries_keep_exact_filter_and_pagination_results() {
    for (sql, expected) in [
        (
            "SELECT count(*) FROM numbers(100) WHERE number >= 20 AND number < 80 AND number != 21 AND number != 79",
            "count\n58\n",
        ),
        (
            "SELECT count(*) FROM generate_series(5, 49, 4) WHERE number >= 13 AND number <= 41 AND number != 25",
            "count\n7\n",
        ),
        (
            "SELECT number FROM numbers(22) WHERE number NOT BETWEEN 4 AND 16 ORDER BY number DESC LIMIT 5 OFFSET 2",
            "number\n19\n18\n17\n3\n2\n",
        ),
        (
            "SELECT number FROM generate_series(1, 100, 7) WHERE number IN (8, 22, 36, 50, 64, 78, 92) ORDER BY number DESC LIMIT 4",
            "number\n92\n78\n64\n50\n",
        ),
    ] {
        let parsed = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
        let mut output = Vec::new();
        execute(&parsed, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
        assert_eq!(String::from_utf8(output).unwrap(), expected, "{sql}");
    }
}
