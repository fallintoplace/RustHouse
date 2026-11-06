use super::*;

#[test]
fn counts_unique_or_matches_on_a_billion_generated_rows() {
    let branch = |value| {
        vec![CompiledPredicate {
            index: 0,
            test: CompiledTest::Comparison {
                operator: Operator::Eq,
                target: Number::Unsigned(value),
            },
        }]
    };
    let branches = vec![
        branch(0),
        branch(4),
        branch(4),
        branch(999_999_999),
        branch(1_000_000_000),
    ];
    assert_eq!(
        count_monotonic_numbers(&branches, 0, 1_000_000_000, 1),
        Some(3)
    );
    assert_eq!(
        count_monotonic_numbers(&[branch(11), branch(15)], 1, 10, 2),
        Some(2)
    );
}
