use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn grouped_count_literals_match_count_star() {
    for literal in ["1", "0", "'x'"] {
        let sql = format!(
            "SELECT number, COUNT({literal}) FROM numbers(4) GROUP BY number ORDER BY number"
        );
        let star = "SELECT number, COUNT(*) FROM numbers(4) GROUP BY number ORDER BY number";
        assert_eq!(run(&sql), run(star), "{sql}");
    }
}

#[test]
fn grouped_count_literal_aliases_match_count_star_aliases() {
    let literal = "SELECT number, COUNT(1) AS total FROM numbers(4) GROUP BY number ORDER BY number";
    let star = "SELECT number, COUNT(*) AS total FROM numbers(4) GROUP BY number ORDER BY number";
    assert_eq!(run(literal), run(star));
}
