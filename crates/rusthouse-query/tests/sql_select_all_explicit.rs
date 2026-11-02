use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn select_all_keeps_duplicate_rows_and_default_projection() {
    let sql = "SELECT ALL number FROM numbers(4) ORDER BY number DESC";
    let default = "SELECT number FROM numbers(4) ORDER BY number DESC";
    assert_eq!(run(sql), "number\n3\n2\n1\n0\n");
    assert_eq!(run(sql), run(default));
    assert!(!parse_select(sql).unwrap().distinct);
}

#[test]
fn explicit_all_is_not_distinct() {
    assert!(parse_select("SELECT DISTINCT number FROM numbers(3)").unwrap().distinct);
    assert!(parse_select("SELECT DISTINCT ALL number FROM numbers(3)").is_err());
}
