use super::*;

#[test]
fn dense_in_matches_the_general_predicate_for_positive_and_negative_bytes() {
    for negated in [false, true] {
        for signed in [false, true] {
            let targets = if signed {
                (-20i128..=20).map(NumberKey::Signed).collect()
            } else {
                (50u128..=80).map(NumberKey::Unsigned).collect()
            };
            let test = CompiledTest::InSet { targets, negated };
            let batch = if signed {
                vec![NumericColumn::Int8(
                    (-128i16..=127)
                        .map(|i| i as i8)
                        .collect::<Vec<_>>()
                        .repeat(4),
                )]
            } else {
                vec![NumericColumn::UInt8(
                    (0..=255).collect::<Vec<u8>>().repeat(4),
                )]
            };
            let expected = (0..batch[0].len())
                .filter(|&row| test.matches(super::super::number_at(&batch[0], row)))
                .count();
            let predicate = CompiledPredicate { index: 0, test };
            assert_eq!(count_batch_matches(&batch, &[vec![predicate]]), expected);
        }
    }
}
