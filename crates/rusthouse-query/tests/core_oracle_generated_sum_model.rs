use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn generated_sum_matches_independent_arithmetic_progression() {
    for rows in [1u64, 2, 17, 1024, 1025, 20_000] {
        let expected = u128::from(rows) * u128::from(rows - 1) / 2;
        let sql = format!("SELECT sum(number) FROM numbers({rows})");
        assert_eq!(run(&sql), format!("sum(number)\n{expected}\n"), "rows={rows}");
    }
}
