use rusthouse_query::{Operator, Predicate, parse_select};

#[test]
fn parses_string_bounds_and_keeps_numeric_between() {
    let query = parse_select(
        "SELECT count(*) FROM parts WHERE p_brand BETWEEN 'Brand#11' AND 'Brand#15'",
    )
    .unwrap();
    assert_eq!(query.predicates.len(), 2);
    assert_eq!(
        query.predicates[0],
        Predicate::Comparison {
            column: "p_brand".into(),
            operator: Operator::Ge,
            value: "Brand#11".into(),
        }
    );
    assert_eq!(
        query.predicates[1],
        Predicate::Comparison {
            column: "p_brand".into(),
            operator: Operator::Le,
            value: "Brand#15".into(),
        }
    );
    assert!(parse_select("SELECT number FROM numbers(10) WHERE number BETWEEN 2 AND 8").is_ok());
}

#[test]
fn parses_not_between_for_dates_and_rejects_missing_bounds() {
    let query = parse_select(
        "SELECT count(*) FROM lineitem WHERE l_shipdate NOT BETWEEN '1994-01-01' AND '1994-12-31'",
    )
    .unwrap();
    assert_eq!(
        query.predicates,
        vec![Predicate::NotBetween {
            column: "l_shipdate".into(),
            lower: "1994-01-01".into(),
            upper: "1994-12-31".into(),
        }]
    );
    assert!(parse_select("SELECT count(*) FROM lineitem WHERE l_shipdate BETWEEN '1994-01-01' AND").is_err());
}
