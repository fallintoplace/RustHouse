use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn standard_aggregate_alias_matches_existing_aggregate() {
    for (standard, existing) in [
        (
            "SELECT VAR_SAMP(number) FROM numbers(4)",
            "SELECT varsamp(number) FROM numbers(4)",
        ),
        (
            "SELECT number, VAR_SAMP(number) FROM numbers(4) GROUP BY number ORDER BY number",
            "SELECT number, varsamp(number) FROM numbers(4) GROUP BY number ORDER BY number",
        ),
        (
            "SELECT VAR_SAMP(number), COUNT(*) FROM numbers(4)",
            "SELECT varsamp(number), COUNT(*) FROM numbers(4)",
        ),
    ] {
        assert_eq!(run(standard), run(existing), "{standard}");
    }
}
