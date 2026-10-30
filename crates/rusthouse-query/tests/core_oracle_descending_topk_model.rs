use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn filtered_descending_topk_matches_independent_integer_model() {
    for rows in [1u64, 65, 257, 2049, 4097] {
        let sql = format!(
            "SELECT number FROM numbers({rows}) WHERE number % 5 = 2 ORDER BY number DESC LIMIT 7 OFFSET 3"
        );
        let mut reference = String::from("number\n");
        for value in (0..rows)
            .rev()
            .filter(|value| value % 5 == 2)
            .skip(3)
            .take(7)
        {
            reference.push_str(&format!("{value}\n"));
        }
        assert_eq!(run(&sql), reference, "rows={rows}");
    }
}
