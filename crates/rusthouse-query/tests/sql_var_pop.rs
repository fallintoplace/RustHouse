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
            "SELECT VAR_POP(number) FROM numbers(4)",
            "SELECT varpop(number) FROM numbers(4)",
        ),
        (
            "SELECT number, VAR_POP(number) FROM numbers(4) GROUP BY number ORDER BY number",
            "SELECT number, varpop(number) FROM numbers(4) GROUP BY number ORDER BY number",
        ),
        (
            "SELECT VAR_POP(number), COUNT(*) FROM numbers(4)",
            "SELECT varpop(number), COUNT(*) FROM numbers(4)",
        ),
    ] {
        assert_eq!(run(standard), run(existing), "{standard}");
    }
}
