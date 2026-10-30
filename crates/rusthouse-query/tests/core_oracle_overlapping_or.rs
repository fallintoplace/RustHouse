use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn overlapping_or_branches_never_double_count_matches() {
    for rows in [1, 4, 11, 1024, 4097] {
        let overlapping = format!(
            "SELECT count(*) FROM numbers({rows}) WHERE number = 3 OR number IN (3, 7, 10) OR number = 7"
        );
        let set = format!(
            "SELECT count(*) FROM numbers({rows}) WHERE number IN (3, 7, 10)"
        );
        assert_eq!(run(&overlapping), run(&set), "rows={rows}");
    }
}
