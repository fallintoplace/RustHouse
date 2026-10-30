use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn or_and_in_match_for_duplicate_disjunctions() {
    for rows in [1, 4, 12, 1025, 4097] {
        let or_query = format!(
            "SELECT number FROM numbers({rows}) WHERE number = 3 OR number = 11 OR number = 11 ORDER BY number DESC"
        );
        let in_query = format!(
            "SELECT number FROM numbers({rows}) WHERE number IN (3, 11) ORDER BY number DESC"
        );
        assert_eq!(run(&or_query), run(&in_query), "rows={rows}");
    }
}
