use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let mut out = Vec::new();
    execute(&parse_select(sql).unwrap(), None, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn order_by_all_expands_selected_column_order() {
    let parsed = parse_select("SELECT number FROM numbers(6) ORDER BY ALL DESC LIMIT 3").unwrap();
    let orders = parsed.order_terms().collect::<Vec<_>>();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].column, "number");
    assert!(orders[0].descending);
    assert_eq!(run("SELECT number FROM numbers(6) ORDER BY ALL DESC LIMIT 3"), "number\n5\n4\n3\n");
}

#[test]
fn order_by_all_expands_aggregate_output_columns() {
    let parsed = parse_select("SELECT number, count(*) FROM numbers(5) GROUP BY number ORDER BY ALL").unwrap();
    let orders = parsed.order_terms().map(|term| term.column.as_str()).collect::<Vec<_>>();
    assert_eq!(orders, ["number", "count"]);
    assert_eq!(run("SELECT number, count(*) FROM numbers(3) GROUP BY number ORDER BY ALL DESC"),
               "number\tcount\n2\t1\n1\t1\n0\t1\n");
}

#[test]
fn order_by_all_rejects_unexpandable_projections_and_clauses() {
    for sql in [
        "SELECT * FROM numbers(3) ORDER BY ALL",
        "SELECT count(*) FROM numbers(3) ORDER BY ALL",
        "SELECT number FROM numbers(3) ORDER BY ALL NULLS",
        "SELECT number FROM numbers(3) ORDER BY ALL, number",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
