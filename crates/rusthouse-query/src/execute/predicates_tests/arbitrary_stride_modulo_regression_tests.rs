use super::*;
#[test]
fn arithmetic_progression_matches_bounded_rowwise_reference() {
    for start in [0u64, 7, 100, u64::MAX - 100] {
        for step in [1u64, 2, 3, 6, 9, u64::MAX] {
            for modulus in 1..=15u64 {
                for target in [0u128, 1, 2, u128::from(modulus - 1), u128::from(modulus)] {
                    for rows in [0u64, 1, 2, 3, 100] {
                        for negated in [false, true] {
                            let predicate = CompiledPredicate {
                                index: 0,
                                test: CompiledTest::Comparison {
                                    operator: if negated {
                                        Operator::ModuloNe(modulus)
                                    } else {
                                        Operator::Modulo(modulus)
                                    },
                                    target: Number::Unsigned(target),
                                },
                            };
                            let expected = (0..rows)
                                .filter(|index| {
                                    let value = u128::from(start)
                                        + u128::from(*index) * u128::from(step);
                                    let matched = value % u128::from(modulus) == target;
                                    matched != negated
                                })
                                .count() as u64;
                            assert_eq!(
                                count_monotonic_numbers(&[vec![predicate]], start, rows, step),
                                Some(expected),
                                "start={start} step={step} modulo={modulus} target={target} rows={rows} negated={negated}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn billion_row_non_unit_stride_is_counted_without_scanning() {
    let predicate = CompiledPredicate {
        index: 0,
        test: CompiledTest::Comparison {
            operator: Operator::Modulo(12),
            target: Number::Unsigned(5),
        },
    };
    assert_eq!(
        count_monotonic_numbers(&[vec![predicate]], 1, 1_000_000_000, 4),
        Some(333_333_333)
    );
}
