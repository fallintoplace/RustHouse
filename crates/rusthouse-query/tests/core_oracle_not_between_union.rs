use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn not_between_matches_disjoint_ranges_at_boundary_rows() {
    for rows in [1, 7, 8, 31, 32, 1025, 4097] {
        let between = format!(
            "SELECT count(*) FROM numbers({rows}) WHERE number NOT BETWEEN 7 AND 31"
        );
        let union = format!(
            "SELECT count(*) FROM numbers({rows}) WHERE number < 7 OR number > 31"
        );
        assert_eq!(run(&between), run(&union), "rows={rows}");
    }
}
