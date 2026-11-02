use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn standard_overlay_replaces_one_based_range() {
    assert_eq!(
        run("SELECT OVERLAY('abcdef' PLACING 'X' FROM 3 FOR 2) AS changed"),
        "changed\nabXef\n"
    );
    assert_eq!(
        run("SELECT OVERLAY('abcdef' PLACING 'X' FROM 3) AS changed"),
        "changed\nabXdef\n"
    );
    assert_eq!(
        run("SELECT OVERLAY('abcdef' PLACING 'X' FROM 3 FOR 2) AS changed"),
        run("SELECT OVERLAY('abcdef', 'X', 3, 2) AS changed")
    );
}

#[test]
fn standard_overlay_rejects_incomplete_syntax() {
    for sql in [
        "SELECT OVERLAY('abc' PLACING 'X' 2)",
        "SELECT OVERLAY('abc' PLACING 'X' FROM)",
        "SELECT OVERLAY('abc' PLACING 'X' FROM 2 FOR)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
