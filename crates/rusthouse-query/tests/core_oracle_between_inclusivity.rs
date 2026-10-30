use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn between_matches_inclusive_bounds_around_scan_edges() {
    for rows in [1, 17, 18, 1024, 1025, 4097] {
        let between = format!(
            "SELECT sum(number) FROM numbers({rows}) WHERE number BETWEEN 17 AND 1024"
        );
        let comparisons = format!(
            "SELECT sum(number) FROM numbers({rows}) WHERE number >= 17 AND number <= 1024"
        );
        assert_eq!(run(&between), run(&comparisons), "rows={rows}");
    }
}
