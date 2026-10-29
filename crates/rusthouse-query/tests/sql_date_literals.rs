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
    assert!(
        matches!(&query.predicates[0], Predicate::Comparison { value, .. } if value == "2020-01-01 00:00:00")
    );
    assert!(parse_select("SELECT count(*) FROM events WHERE created_at >= DATE 20200101").is_err());
    assert!(parse_select("SELECT count(*) FROM events WHERE created_at >= TIMESTAMP").is_err());
}

#[test]
fn parses_tpch_q6_style_date_intervals_and_decimal_bounds() {
    let select = parse_select(
        "SELECT SUM(l_extendedprice * l_discount) AS revenue FROM lineitem \
         WHERE l_shipdate >= DATE '1994-01-01' \
         AND l_shipdate < DATE '1994-01-01' + INTERVAL '1' YEAR \
         AND l_discount BETWEEN 0.06 - 0.01 AND 0.06 + 0.01 \
         AND l_quantity < 24",
    )
    .unwrap();
    assert_eq!(select.predicates.len(), 5);
    assert_eq!(
        select.predicates[1],
        Predicate::Comparison {
            column: "l_shipdate".into(),
            operator: Operator::Lt,
            value: "1995-01-01".into(),
        }
    );
    assert_eq!(
        select.predicates[2],
        Predicate::Comparison {
            column: "l_discount".into(),
            operator: Operator::Ge,
            value: "0.05".into(),
        }
    );
    assert_eq!(
        select.predicates[3],
        Predicate::Comparison {
            column: "l_discount".into(),
            operator: Operator::Le,
            value: "0.07".into(),
        }
    );
}

#[test]
fn rejects_invalid_date_arithmetic_without_accepting_partial_sql() {
    for statement in [
        "SELECT count(*) FROM lineitem WHERE l_shipdate < DATE '1994-01-01' + 1",
        "SELECT count(*) FROM lineitem WHERE l_shipdate < DATE '1994-01-01' + INTERVAL '1' WEEK",
        "SELECT count(*) FROM lineitem WHERE l_shipdate < DATE '1994-01-01' + INTERVAL 'bad' YEAR",
        "SELECT count(*) FROM lineitem WHERE l_discount BETWEEN 0.06 + 'bad' AND 0.07",
    ] {
        assert!(parse_select(statement).is_err(), "{statement}");
    }
}
