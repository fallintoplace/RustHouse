use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn standard_stddev_samp_matches_existing_aggregate() {
    for (standard, existing) in [
        (
            "SELECT STDDEV_SAMP(number) FROM numbers(4)",
            "SELECT stddevSamp(number) FROM numbers(4)",
        ),
        (
            "SELECT number, STDDEV_SAMP(number) FROM numbers(4) GROUP BY number ORDER BY number",
            "SELECT number, stddevSamp(number) FROM numbers(4) GROUP BY number ORDER BY number",
        ),
        (
            "SELECT STDDEV_SAMP(number), COUNT(*) FROM numbers(4)",
            "SELECT stddevSamp(number), COUNT(*) FROM numbers(4)",
        ),
    ] {
        assert_eq!(run(standard), run(existing), "{standard}");
    }
}
