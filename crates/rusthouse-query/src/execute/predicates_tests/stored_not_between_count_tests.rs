use super::*;

#[test]
fn matches_row_wise_not_between_for_signed_and_nan_values() {
    let signed = vec![NumericColumn::Int64((-1_000..1_000).collect())];
    let floats = vec![NumericColumn::Float64(vec![
        f64::NAN,
        f64::NEG_INFINITY,
        -1.0,
        0.0,
        1.0,
        f64::INFINITY,
    ])];
    for (batch, lower, upper) in [
        (&signed, Number::Signed(-10), Number::Signed(10)),
        (&floats, Number::Float(-1.0), Number::Float(1.0)),
        (&floats, Number::Float(1.0), Number::Float(-1.0)),
    ] {
        let branches = vec![vec![CompiledPredicate {
            index: 0,
            test: CompiledTest::NotBetween { lower, upper },
        }]];
        let expected = (0..batch[0].len())
            .filter(|&row| batch_matches(batch, row, &branches))
            .count();
        assert_eq!(count_batch_matches(batch, &branches), expected);
    }
}
