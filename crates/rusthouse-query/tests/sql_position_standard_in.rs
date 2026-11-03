use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn position_in_syntax_finds_substring_one_based() {
    assert_eq!(run("SELECT POSITION('na' IN 'banana') AS pos"), "pos\n3\n");
    assert_eq!(run("SELECT POSITION('missing' IN 'banana') AS pos"), "pos\n0\n");
    assert_eq!(run("SELECT POSITION('' IN 'banana') AS pos"), "pos\n1\n");
}

#[test]
fn original_comma_syntax_keeps_its_start_argument() {
    assert_eq!(run("SELECT POSITION('banana', 'na', 4) AS pos"), "pos\n5\n");
    assert!(parse_select("SELECT POSITION('na' IN 'banana', 4)").is_err());
}
