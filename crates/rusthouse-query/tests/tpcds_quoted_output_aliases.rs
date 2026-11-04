use rusthouse_query::{execute, parse_join, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn quoted_alias_with_spaces_names_output_and_orders() {
    assert_eq!(
        run(
            "SELECT number AS \"order count\" FROM numbers(4) ORDER BY \"order count\" DESC LIMIT 2"
        ),
        "order count\n3\n2\n"
    );
    assert_eq!(
        run("SELECT COUNT(*) AS \"order count\" FROM numbers(4)"),
        "order count\n4\n"
    );
}

#[test]
fn grouped_aggregate_keeps_quoted_output_name() {
    assert_eq!(
        run(
            "SELECT number, COUNT(*) AS \"order count\" FROM numbers(2) GROUP BY number ORDER BY number"
        ),
        "number\torder count\n0\t1\n1\t1\n"
    );
    assert!(parse_select("SELECT number AS \"order count\" FROM numbers(1)").is_ok());
}

#[test]
fn multiple_aggregates_keep_quoted_output_names() {
    assert_eq!(
        run("SELECT COUNT(*) AS \"order count\", SUM(number) AS \"number sum\" FROM numbers(4)"),
        "order count\tnumber sum\n4\t6\n"
    );
}

#[test]
fn joined_group_aliases_with_spaces_are_output_only() {
    let query = parse_join(
        "SELECT l.region AS \"region name\", COUNT(*) AS \"order count\" FROM lefts l INNER JOIN rights r ON l.id = r.id GROUP BY l.region",
    )
    .unwrap()
    .unwrap();
    assert_eq!(query.aggregate_alias.as_deref(), Some("order count"));
    assert_eq!(query.grouped_key_aliases, vec![(0, "region name".into())]);
}

#[test]
fn multi_word_names_are_not_storage_identifiers() {
    assert!(parse_select("SELECT \"order count\" FROM numbers(1)").is_err());
    assert!(parse_select("SELECT number FROM \"table name\"").is_err());
    assert!(parse_select("SELECT number AS \"order\tcount\" FROM numbers(1)").is_err());
    assert!(parse_select("SELECT number AS \"order\\ncount\" FROM numbers(1)").is_err());
}
