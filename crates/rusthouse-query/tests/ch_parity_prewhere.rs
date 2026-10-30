use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut result = Vec::new();
    execute(&query, None, &mut result).unwrap();
    String::from_utf8(result).unwrap()
}

#[test]
fn prewhere_filters_generated_rows() {
    assert_eq!(
        run("SELECT number FROM numbers(9) PREWHERE number % 2 = 0 ORDER BY number"),
        "number\n0\n2\n4\n6\n8\n"
    );
}

#[test]
fn prewhere_and_where_preserve_boolean_precedence() {
    assert_eq!(
        run("SELECT number FROM numbers(10) PREWHERE (number = 1 OR number = 2 OR number = 3) WHERE (number = 2 OR number = 4) ORDER BY number"),
        "number\n2\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(10) PREWHERE number >= 3 WHERE number < 6 ORDER BY number"),
        "number\n3\n4\n5\n"
    );
}

#[test]
fn prewhere_applies_to_grouped_aggregates() {
    assert_eq!(
        run("SELECT count(*) FROM numbers(10) PREWHERE number >= 2 WHERE number < 7"),
        "count\n5\n"
    );
}

#[test]
fn prewhere_rejects_misplaced_and_unbounded_filters() {
    for sql in [
        "SELECT 1 PREWHERE 1 = 1",
        "SELECT number FROM numbers(4) PREWHERE",
        "SELECT number FROM numbers(4) WHERE number > 1 PREWHERE number < 3",
        "SELECT number FROM numbers(4) PREWHERE number > 1 PREWHERE number < 3",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
    let p = (0..9).map(|i| format!("number = {i}")).collect::<Vec<_>>().join(" OR ");
    let w = (0..8).map(|i| format!("number = {i}")).collect::<Vec<_>>().join(" OR ");
    let too_many = format!("SELECT number FROM numbers(10) PREWHERE ({p}) WHERE ({w})");
    assert!(parse_select(&too_many).is_err());
}
