use super::*;

#[test]
fn float32_and_float64_match_generic_predicate_semantics() {
    let cases = vec![
        -0.0f64,
        0.0,
        -1.0,
        1.0,
        0.5,
        -0.5,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        12.25,
        1.0 / 3.0,
        12345.0,
    ];
    for negated in [false, true] {
        let mut targets = cases.iter().copied().map(Number::Float).collect::<Vec<_>>();
        targets.extend((0..30).map(|value| Number::Float(value as f64 * 0.25)));
        let test = compile_in_test(targets, negated);
        let input32 = cases
            .iter()
            .copied()
            .map(|value| value as f32)
            .collect::<Vec<_>>();
        let input64 = cases.clone();
        let reference32 = input32
            .iter()
            .filter(|&&value| test.matches(Number::Float(f64::from(value))))
            .count();
        let reference64 = input64
            .iter()
            .filter(|&&value| test.matches(Number::Float(value)))
            .count();
        let branch = [vec![CompiledPredicate { index: 0, test }]];
        assert_eq!(
            count_batch_matches(&[NumericColumn::Float32(input32)], &branch),
            reference32
        );
        assert_eq!(
            count_batch_matches(&[NumericColumn::Float64(input64)], &branch),
            reference64
        );
    }
}
