use super::*;

#[test]
fn ordered_and_unordered_timestamp_inventories_match_reference() {
    let ordered = (1..=16_384u64)
        .map(|generation| (generation, generation * 5))
        .collect::<Vec<_>>();
    let mut descending = ordered.clone();
    descending.reverse();
    let now = 16_384 * 5;
    for minimum_age in [0, 5, 500, 25_000, 81_920, u64::MAX] {
        let expected = ordered
            .iter()
            .filter(|&&(generation, created_at)| {
                generation <= 16_384 - 128
                    && generation != 13
                    && generation != 917
                    && now - created_at >= minimum_age
            })
            .map(|&(generation, _)| generation)
            .collect::<Vec<_>>();
        for inventory in [&ordered, &descending] {
            let selected = retention_candidates_with_min_age(
                inventory,
                16_384,
                128,
                &[13, 917],
                &[16_384],
                now,
                minimum_age,
            )
            .unwrap();
            assert_eq!(selected, expected, "age={minimum_age}");
        }
    }
}

#[test]
fn duplicate_timestamps_and_future_generations_still_fail() {
    assert!(
        retention_candidates_with_min_age(&[(2, 20), (1, 10), (1, 10)], 2, 1, &[], &[2], 20, 0)
            .is_err()
    );
    assert!(
        retention_candidates_with_min_age(&[(1, 10), (2, 21)], 2, 1, &[], &[2], 20, 0).is_err()
    );
}
