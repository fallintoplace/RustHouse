use super::*;

#[test]
fn matches_signed_and_unsigned_modulo_counts() {
    let unsigned = vec![NumericColumn::UInt64((0..10_000).collect())];
    let signed = vec![NumericColumn::Int64((-5_000..5_000).collect())];
    for (batch, target, modulus) in [
        (&unsigned, Number::Unsigned(2), 7u64),
        (&signed, Number::Signed(-2), 7u64),
    ] {
        for negated in [false, true] {
            let branches = vec![vec![CompiledPredicate {
                index: 0,
                test: CompiledTest::Comparison {
                    operator: if negated {
                        Operator::ModuloNe(modulus)
                    } else {
                        Operator::Modulo(modulus)
                    },
                    target,
                },
            }]];
            let expected = (0..batch[0].len())
                .filter(|&row| batch_matches(batch, row, &branches))
                .count();
            assert_eq!(count_batch_matches(batch, &branches), expected);
        }
    }
}
