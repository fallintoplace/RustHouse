use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn count_nonnullable_number_matches_count_star_with_filters() {
    for rows in [1u64, 2, 64, 1025, 4097] {
        let column = format!(
            "SELECT count(number) FROM numbers({rows}) WHERE number % 7 = 3"
        );
        let all = format!(
            "SELECT count(*) FROM numbers({rows}) WHERE number % 7 = 3"
        );
        let left = run(&column);
        let right = run(&all);
        assert_eq!(left.lines().nth(1), right.lines().nth(1), "rows={rows}");
    }
}
