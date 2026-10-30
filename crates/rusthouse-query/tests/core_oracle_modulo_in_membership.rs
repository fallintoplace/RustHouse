use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn modulo_matches_enumerated_membership_on_generated_numbers() {
    let values = (0u64..100)
        .filter(|value| value % 3 == 1)
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    for rows in [1, 7, 30, 100] {
        let modulo = format!(
            "SELECT count(*) FROM numbers({rows}) WHERE number % 3 = 1"
        );
        let members = format!(
            "SELECT count(*) FROM numbers({rows}) WHERE number IN ({values})"
        );
        assert_eq!(run(&modulo), run(&members), "rows={rows}");
    }
}
