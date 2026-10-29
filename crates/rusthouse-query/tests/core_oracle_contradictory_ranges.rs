use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn contradictory_numeric_predicates_keep_count_at_zero() {
    for rows in [1, 9, 20, 1024, 4097, 1_000_000] {
        let sql = format!(
            "SELECT count(*) FROM numbers({rows}) WHERE number >= 20 AND number < 10"
        );
        assert_eq!(run(&sql), "count\n0\n", "rows={rows}");
    }
}
