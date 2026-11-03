use rusthouse_query::{execute, parse_join, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn single_projection_alias_without_as_renames_output_and_binds_order() {
    assert_eq!(
        run("SELECT number value FROM numbers(4) ORDER BY value DESC LIMIT 2"),
        "value\n3\n2\n"
    );
    assert_eq!(
        run("SELECT number AS value FROM numbers(3) ORDER BY value DESC LIMIT 1"),
        "value\n2\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(2)"),
        "number\n0\n1\n"
    );
}

#[test]
fn grouped_metrics_allow_implicit_aliases_and_reject_duplicates() {
    assert_eq!(
        run("SELECT number, COUNT(*) row_count, SUM(number) total FROM numbers(3) GROUP BY number ORDER BY number"),
        "number\trow_count\ttotal\n0\t1\t0\n1\t1\t1\n2\t1\t2\n"
    );
    assert!(
        parse_select(
            "SELECT number, SUM(number) metric, COUNT(*) METRIC FROM numbers(3) GROUP BY number"
        )
        .is_err()
    );
}

#[test]
fn grouped_join_keys_and_metrics_allow_implicit_aliases() {
    let query = parse_join(
        "SELECT l.region region_name, COUNT(*) matches FROM lefts l INNER JOIN rights r ON l.id = r.id GROUP BY region_name ORDER BY region_name",
    )
    .unwrap()
    .unwrap();
    assert_eq!(query.grouped_key_aliases, vec![(0, "region_name".into())]);
    assert_eq!(query.aggregate_alias.as_deref(), Some("matches"));
}
