use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn filtered_sum_matches_reference_modular_reduction() {
    for rows in [4u64, 17, 1025, 4097, 20_000] {
        let expected: u128 = (0..rows)
            .filter(|value| value % 11 == 3)
            .map(u128::from)
            .sum();
        let sql = format!(
            "SELECT sum(number) FROM numbers({rows}) WHERE number % 11 = 3"
        );
        assert_eq!(run(&sql), format!("sum(number)\n{expected}\n"), "rows={rows}");
    }
}
