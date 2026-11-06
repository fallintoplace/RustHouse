use super::*;

#[test]
fn between_matches_two_independent_comparisons_at_boundaries_and_nulls() {
    let predicates = vec![
        Predicate::Comparison {
            column: "amount".into(),
            operator: Operator::Ge,
            value: "1000".into(),
        },
        Predicate::Comparison {
            column: "amount".into(),
            operator: Operator::Le,
            value: "9000".into(),
        },
    ];
    let indices = HashMap::from([("amount".to_owned(), 0usize)]);
    let types = HashMap::from([("amount".to_owned(), "Nullable(UInt16)".to_owned())]);
    let conditions = compile(&predicates, &indices, &types).unwrap();
    assert_eq!(conditions.len(), 1);
    assert!(matches!(conditions[0].test, Test::NumericBetween(..)));
    let batch = [TypedColumn::NullableNumeric {
        values: NumericColumn::UInt16(vec![999, 1000, 9000, 9001, 5000]),
        nulls: vec![0, 0, 0, 0, 1],
    }];
    let old = vec![
        Condition {
            index: 0,
            test: Test::Compare(Operator::Ge, Cell::Numeric(Number::Unsigned(1000))),
        },
        Condition {
            index: 0,
            test: Test::Compare(Operator::Le, Cell::Numeric(Number::Unsigned(9000))),
        },
    ];
    for row in 0..5 {
        assert_eq!(
            passes(&batch, row, std::slice::from_ref(&conditions)),
            passes(&batch, row, std::slice::from_ref(&old)),
            "row {row}"
        );
    }
}

#[test]
fn fused_float_between_preserves_nan_semantics() {
    let predicates = vec![
        Predicate::Comparison {
            column: "x".into(),
            operator: Operator::Ge,
            value: "-1".into(),
        },
        Predicate::Comparison {
            column: "x".into(),
            operator: Operator::Le,
            value: "1".into(),
        },
    ];
    let indices = HashMap::from([("x".to_owned(), 0usize)]);
    let types = HashMap::from([("x".to_owned(), "Float64".to_owned())]);
    let conditions = compile(&predicates, &indices, &types).unwrap();
    let batch = [TypedColumn::Numeric(NumericColumn::Float64(vec![
        f64::NAN,
        -1.0,
        0.0,
        1.0,
        2.0,
    ]))];
    let results = (0..5)
        .map(|row| passes(&batch, row, std::slice::from_ref(&conditions)))
        .collect::<Vec<_>>();
    assert_eq!(results, vec![false, true, true, true, false]);
}
