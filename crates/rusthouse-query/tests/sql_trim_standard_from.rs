use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn standard_trim_supports_leading_trailing_and_both() {
    assert_eq!(run("SELECT TRIM(LEADING 'x' FROM 'xxabxx') AS t"), "t\nabxx\n");
    assert_eq!(run("SELECT TRIM(TRAILING 'x' FROM 'xxabxx') AS t"), "t\nxxab\n");
    assert_eq!(run("SELECT TRIM(BOTH 'x' FROM 'xxabxx') AS t"), "t\nab\n");
    assert_eq!(run("SELECT TRIM(BOTH FROM '  ab  ') AS t"), "t\nab\n");
    assert_eq!(run("SELECT TRIM('x' FROM 'xxabxx') AS t"), "t\nab\n");
}

#[test]
fn existing_comma_trim_and_missing_from_are_handled() {
    assert_eq!(run("SELECT TRIM('xxabxx', 'x') AS t"), "t\nab\n");
    assert_eq!(run("SELECT TRIM('  ab  ') AS t"), "t\nab\n");
    assert!(parse_select("SELECT TRIM(LEADING 'x' 'xxabxx')").is_err());
}
