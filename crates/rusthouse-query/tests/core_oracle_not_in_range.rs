use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn not_in_exclusion_matches_disjoint_range_filter() {
    for rows in [1, 3, 17, 64, 1024] {
        let excluded = format!(
            "SELECT number FROM numbers({rows}) WHERE number NOT IN (3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16) ORDER BY number DESC LIMIT 40"
        );
        let bounds = format!(
            "SELECT number FROM numbers({rows}) WHERE number < 3 OR number > 16 ORDER BY number DESC LIMIT 40"
        );
        assert_eq!(run(&excluded), run(&bounds), "rows={rows}");
    }
}
