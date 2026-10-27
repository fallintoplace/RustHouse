use rusthouse_query::{Operator, Predicate, parse_select};

#[test]
fn accepts_typed_date_literals_in_comparisons() {
    let select = parse_select(
        "SELECT count(*) FROM lineitem WHERE l_shipdate >= DATE '1994-01-01' AND l_shipdate < DATE '1995-01-01'",
    )
    .unwrap();
    assert_eq!(select.predicates.len(), 2);
    assert_eq!(
        select.predicates[0],
        Predicate::Comparison {
            column: "l_shipdate".into(),
            operator: Operator::Ge,
            value: "1994-01-01".into(),
        }
    );
    assert_eq!(
        select.predicates[1],
        Predicate::Comparison {
            column: "l_shipdate".into(),
            operator: Operator::Lt,
            value: "1995-01-01".into(),
        }
    );
}

#[test]
fn accepts_typed_timestamp_literals_and_rejects_unquoted_values() {
    let query = parse_select(
        "SELECT count(*) FROM events WHERE created_at >= TIMESTAMP '2020-01-01 00:00:00'",
    )
    .unwrap();
    assert!(matches!(&query.predicates[0], Predicate::Comparison { value, .. } if value == "2020-01-01 00:00:00"));
    assert!(parse_select("SELECT count(*) FROM events WHERE created_at >= DATE 20200101").is_err());
    assert!(parse_select("SELECT count(*) FROM events WHERE created_at >= TIMESTAMP").is_err());
}
