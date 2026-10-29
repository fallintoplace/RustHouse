use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn ascending_range_topk_matches_independent_integer_model() {
    for rows in [1u64, 23, 65, 334, 2049, 4097] {
        let sql = format!(
            "SELECT number FROM numbers({rows}) WHERE number >= 23 AND number <= 333 ORDER BY number LIMIT 11 OFFSET 17"
        );
        let mut reference = String::from("number\n");
        for value in (0..rows)
            .filter(|value| (23..=333).contains(value))
            .skip(17)
            .take(11)
        {
            reference.push_str(&format!("{value}\n"));
        }
        assert_eq!(run(&sql), reference, "rows={rows}");
    }
}
