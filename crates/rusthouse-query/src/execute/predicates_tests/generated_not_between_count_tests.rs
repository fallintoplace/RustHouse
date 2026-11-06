use super::*;

#[test]
fn handles_billion_rows_step_and_reversed_bounds() {
    let count = |low, high, start, rows, step| {
        count_monotonic_numbers(
            &[vec![CompiledPredicate {
                index: 0,
                test: CompiledTest::NotBetween {
                    lower: Number::Unsigned(low),
                    upper: Number::Unsigned(high),
                },
            }]],
            start,
            rows,
            step,
        )
    };
    assert_eq!(
        count(100, 899_999_999, 0, 1_000_000_000, 1),
        Some(100_000_100)
    );
    assert_eq!(count(3, 7, 1, 5, 2), Some(2));
    assert_eq!(count(9, 4, 0, 1_000_000_000, 1), Some(1_000_000_000));
}
