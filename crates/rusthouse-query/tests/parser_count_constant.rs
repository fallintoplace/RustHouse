use rusthouse_query::{execute, parse_select, Projection};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn count_of_numeric_constants_counts_all_rows() {
    for value in ["1", "0", "42"] {
        let sql = format!("SELECT COUNT({value}) FROM numbers(5)");
        assert!(matches!(parse_select(&sql).unwrap().projection, Projection::Count));
        assert_eq!(run(&sql), run("SELECT COUNT(*) FROM numbers(5)"), "{sql}");
    }
}

#[test]
fn count_of_constant_rejects_extra_arguments() {
    assert!(parse_select("SELECT COUNT(1, 2) FROM numbers(5)").is_err());
    assert!(parse_select("SELECT COUNT(1 FROM numbers(5)").is_err());
}
