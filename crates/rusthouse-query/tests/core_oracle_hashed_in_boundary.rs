use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn hashed_in_matches_range_at_short_and_long_scan_lengths() {
    for rows in [1, 15, 16, 17, 1024, 4097] {
        let members = format!(
            "SELECT count(*) FROM numbers({rows}) WHERE number IN (0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15)"
        );
        let bounds = format!(
            "SELECT count(*) FROM numbers({rows}) WHERE number BETWEEN 0 AND 15"
        );
        assert_eq!(run(&members), run(&bounds), "rows={rows}");
    }
}
