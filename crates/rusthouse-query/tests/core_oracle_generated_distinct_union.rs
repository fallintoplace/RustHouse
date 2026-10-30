use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn distinct_matches_count_of_unique_generated_members() {
    for rows in [1u64, 4, 8, 12, 4097] {
        let distinct = format!(
            "SELECT count(DISTINCT number) FROM numbers({rows}) WHERE number IN (0, 3, 7, 7, 10) OR number = 3"
        );
        let regular = format!(
            "SELECT count(*) FROM numbers({rows}) WHERE number IN (0, 3, 7, 10)"
        );
        let left = run(&distinct);
        let right = run(&regular);
        assert_eq!(left.lines().nth(1), right.lines().nth(1), "rows={rows}");
    }
}
