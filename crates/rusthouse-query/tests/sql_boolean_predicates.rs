use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn boolean_predicate_literals_use_zero_and_one() {
    for (boolean, number) in [
        (
            "SELECT number FROM numbers(4) WHERE number = TRUE ORDER BY number",
            "SELECT number FROM numbers(4) WHERE number = 1 ORDER BY number",
        ),
        (
            "SELECT number FROM numbers(4) WHERE number != FALSE ORDER BY number",
            "SELECT number FROM numbers(4) WHERE number != 0 ORDER BY number",
        ),
        (
            "SELECT number FROM numbers(4) WHERE number IN (FALSE, TRUE) ORDER BY number",
            "SELECT number FROM numbers(4) WHERE number IN (0, 1) ORDER BY number",
        ),
        (
            "SELECT number FROM numbers(4) WHERE number BETWEEN FALSE AND TRUE ORDER BY number",
            "SELECT number FROM numbers(4) WHERE number BETWEEN 0 AND 1 ORDER BY number",
        ),
    ] {
        assert_eq!(run(boolean), run(number), "{boolean}");
    }
}
