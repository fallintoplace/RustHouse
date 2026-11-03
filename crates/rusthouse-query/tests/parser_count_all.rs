use rusthouse_query::{execute, parse_select, Projection};

fn run(sql: &str) -> String {
    let mut output = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn count_all_is_equivalent_to_unqualified_count() {
    let query = parse_select("SELECT COUNT(ALL number) FROM numbers(5)").unwrap();
    assert!(matches!(query.projection, Projection::CountColumn { .. }));
    assert_eq!(
        run("SELECT COUNT(ALL number) FROM numbers(5)"),
        run("SELECT COUNT(number) FROM numbers(5)")
    );
    assert_eq!(
        run("SELECT COUNT(ALL *) FROM numbers(5)"),
        run("SELECT COUNT(*) FROM numbers(5)")
    );
}

#[test]
fn count_all_rejects_missing_arguments() {
    assert!(parse_select("SELECT COUNT(ALL) FROM numbers(5)").is_err());
}
