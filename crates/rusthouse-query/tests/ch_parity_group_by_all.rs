use rusthouse_query::{execute, parse_select};

fn output(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut out = Vec::new();
    execute(&query, None, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn group_by_all_infers_explicit_aggregate_group_keys() {
    let explicit = parse_select(
        "SELECT number, count(*) FROM numbers(5) GROUP BY number ORDER BY number"
    ).unwrap();
    let inferred = parse_select(
        "SELECT number, count(*) FROM numbers(5) GROUP BY ALL ORDER BY number"
    ).unwrap();
    assert_eq!(inferred.projection, explicit.projection);
    assert_eq!(inferred.order_by, explicit.order_by);
    assert_eq!(
        output("SELECT number, count(*) FROM numbers(4) GROUP BY ALL ORDER BY number"),
        "number\tcount\n0\t1\n1\t1\n2\t1\n3\t1\n"
    );
}

#[test]
fn group_by_all_lowers_plain_columns_to_distinct() {
    let query = parse_select("SELECT number FROM numbers(5) GROUP BY ALL").unwrap();
    assert!(query.distinct);
    assert_eq!(output("SELECT number FROM numbers(4) GROUP BY ALL ORDER BY number"), "number\n0\n1\n2\n3\n");
}

#[test]
fn group_by_all_rejects_invalid_explicit_mixes() {
    assert_eq!(output("SELECT count(*) FROM numbers(5) GROUP BY ALL"), "count\n5\n");
    for sql in [
        "SELECT number FROM numbers(5) GROUP BY ALL, number",
        "SELECT number, count(*) FROM numbers(5) GROUP BY ALL, number",
        "SELECT 1 GROUP BY ALL",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
