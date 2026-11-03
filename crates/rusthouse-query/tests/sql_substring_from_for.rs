use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn standard_substring_from_for_matches_comma_form() {
    assert_eq!(
        run("SELECT SUBSTRING('abcdef' FROM 2 FOR 3) AS part"),
        "part\nbcd\n"
    );
    assert_eq!(
        run("SELECT SUBSTRING('abcdef' FROM 4) AS part"),
        "part\ndef\n"
    );
    assert_eq!(
        run("SELECT SUBSTRING('abcdef' FROM 2 FOR 3) AS part"),
        run("SELECT SUBSTRING('abcdef', 2, 3) AS part")
    );
}

#[test]
fn incomplete_standard_substring_is_rejected() {
    for sql in [
        "SELECT SUBSTRING('abc' FROM) AS part",
        "SELECT SUBSTRING('abc' FOR 2) AS part",
        "SELECT SUBSTRING('abc' FROM 1 FOR) AS part",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
