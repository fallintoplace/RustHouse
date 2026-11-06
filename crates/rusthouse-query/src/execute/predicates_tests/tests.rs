use super::*;

#[test]
fn signed_int16_bitmap_in_preserves_negative_values_and_negation() {
    let values = (-2048..2048).map(|value| value as i16).collect::<Vec<_>>();
    let targets = (-150..150)
        .map(|value| Number::Signed(i128::from(value)))
        .collect::<Vec<_>>();
    for negated in [false, true] {
        let predicate = CompiledPredicate {
            index: 0,
            test: compile_in_test(targets.clone(), negated),
        };
        let expected = values
            .iter()
            .filter(|&&value| (-150..150).contains(&i32::from(value)) != negated)
            .count();
        assert_eq!(
            count_batch_matches(&[NumericColumn::Int16(values.clone())], &[vec![predicate]]),
            expected
        );
    }
}

#[test]
fn large_unsigned_sixteen_in_matches_hash_membership() {
    let values = (0..4096).map(|value| value as u16).collect::<Vec<_>>();
    let targets = (0..100)
        .map(|value| Number::Unsigned(value * 2))
        .collect::<Vec<_>>();
    for negated in [false, true] {
        let predicate = CompiledPredicate {
            index: 0,
            test: compile_in_test(targets.clone(), negated),
        };
        let expected = values
            .iter()
            .filter(|&&value| (value < 200 && value % 2 == 0) != negated)
            .count();
        assert_eq!(
            count_batch_matches(&[NumericColumn::UInt16(values.clone())], &[vec![predicate]]),
            expected
        );
    }
}

#[test]
fn compiles_fast_numeric_predicates_ahead_of_modulo() {
    let query = crate::parse_select(
        "SELECT number FROM numbers(100) WHERE number % 10 = 0 AND number < 50",
    )
    .unwrap();
    let branches = compile_numbers_predicates(&query).unwrap();
    assert_eq!(branches.len(), 1);
    assert!(matches!(
        &branches[0][0].test,
        CompiledTest::Comparison {
            operator: Operator::Lt,
            ..
        }
    ));
    assert!(number_matches_all(Number::Unsigned(40), &branches));
    assert!(!number_matches_all(Number::Unsigned(60), &branches));
}

#[test]
fn direct_pair_counts_match_rowwise_across_numeric_types() {
    for (values, target_a, target_b) in [
        (
            NumericColumn::UInt16(vec![0, 1, 5, 10, 65535]),
            Number::Unsigned(1),
            Number::Unsigned(11),
        ),
        (
            NumericColumn::Int16(vec![-10, -1, 0, 5, 32767]),
            Number::Signed(-1),
            Number::Signed(6),
        ),
        (
            NumericColumn::Float32(vec![-1.0, 0.0, 0.5, 6.0, f32::NAN]),
            Number::Float(0.0),
            Number::Float(6.0),
        ),
    ] {
        let predicates = vec![vec![
            CompiledPredicate {
                index: 0,
                test: CompiledTest::Comparison {
                    operator: Operator::Ge,
                    target: target_a,
                },
            },
            CompiledPredicate {
                index: 0,
                test: CompiledTest::Comparison {
                    operator: Operator::Lt,
                    target: target_b,
                },
            },
        ]];
        let expected = (0..values.len())
            .filter(|&row| batch_matches(std::slice::from_ref(&values), row, &predicates))
            .count();
        assert_eq!(count_batch_matches(&[values], &predicates), expected);
    }
}

#[test]
fn compiles_or_branches_using_the_same_numeric_conversion() {
    let query =
        crate::parse_select("SELECT number FROM numbers(10) WHERE number = 1 OR number = 3")
            .unwrap();
    let branches = compile_numbers_predicates(&query).unwrap();
    assert_eq!(branches.len(), 2);
    assert!(number_matches_all(Number::Unsigned(1), &branches));
    assert!(number_matches_all(Number::Unsigned(3), &branches));
    assert!(!number_matches_all(Number::Unsigned(2), &branches));
}

#[test]
fn hashed_in_and_not_in_keep_membership_semantics() {
    let values = (0..16).map(Number::Unsigned).collect::<Vec<_>>();
    let included = compile_in_test(values.clone(), false);
    let excluded = compile_in_test(values, true);
    assert!(matches!(&included, CompiledTest::InSet { .. }));
    assert!(matches!(&excluded, CompiledTest::InSet { .. }));
    assert!(included.matches(Number::Unsigned(0)));
    assert!(included.matches(Number::Unsigned(15)));
    assert!(!included.matches(Number::Unsigned(16)));
    assert!(!excluded.matches(Number::Unsigned(0)));
    assert!(excluded.matches(Number::Unsigned(16)));
}

#[test]
fn batched_counts_match_rowwise_predicates_across_numeric_types() {
    for (batch, target) in [
        (NumericColumn::UInt8(vec![0, 1, 3, 5]), Number::Unsigned(3)),
        (
            NumericColumn::UInt64(vec![0, 1, 3, u64::MAX]),
            Number::Unsigned(3),
        ),
        (NumericColumn::Int64(vec![-9, -1, 3, 7]), Number::Signed(3)),
        (
            NumericColumn::Float64(vec![f64::NAN, -1.0, 3.0, 5.0]),
            Number::Float(3.0),
        ),
    ] {
        for operator in [
            Operator::Eq,
            Operator::Ne,
            Operator::Lt,
            Operator::Le,
            Operator::Gt,
            Operator::Ge,
        ] {
            let branches = vec![vec![CompiledPredicate {
                index: 0,
                test: CompiledTest::Comparison { operator, target },
            }]];
            let expected = (0..batch.len())
                .filter(|&row| batch_matches(std::slice::from_ref(&batch), row, &branches))
                .count();
            assert_eq!(
                count_batch_matches(std::slice::from_ref(&batch), &branches),
                expected
            );
        }
    }
}

#[test]
fn batched_counts_preserve_or_and_modulo_fallback() {
    let batch = [NumericColumn::UInt64(vec![0, 1, 2, 3, 4, 5])];
    let make = |operator, target| CompiledPredicate {
        index: 0,
        test: CompiledTest::Comparison {
            operator,
            target: Number::Unsigned(target),
        },
    };
    for branches in [
        vec![vec![make(Operator::Modulo(2), 1)]],
        vec![vec![make(Operator::Lt, 2)], vec![make(Operator::Ge, 4)]],
        vec![vec![make(Operator::Gt, 1), make(Operator::Lt, 5)]],
    ] {
        let expected = (0..batch[0].len())
            .filter(|&row| batch_matches(&batch, row, &branches))
            .count();
        assert_eq!(count_batch_matches(&batch, &branches), expected);
    }
    assert_eq!(count_batch_matches(&batch, &[vec![]]), 6);
}

#[test]
fn hashed_float_in_normalizes_signed_zero_but_not_nan() {
    let values = (1..16)
        .map(|value| Number::Float(f64::from(value)))
        .chain(std::iter::once(Number::Float(-0.0)))
        .collect();
    let included = compile_in_test(values, false);
    assert!(included.matches(Number::Float(0.0)));
    assert!(!included.matches(Number::Float(f64::NAN)));
}
