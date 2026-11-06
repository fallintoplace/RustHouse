use super::*;

#[test]
fn chronological_and_unordered_large_pin_sets_produce_identical_budgets() {
    let entries = (1..=16_384u64)
        .map(|generation| (generation, generation.wrapping_mul(17) % 997 + 1))
        .collect::<Vec<_>>();
    let ordered_pins = (1..=16_384u64).step_by(9).collect::<Vec<_>>();
    let mut reversed_pins = ordered_pins.clone();
    reversed_pins.reverse();
    for floor in [1u64, 7_777, 16_385] {
        for budget in [0u64, 6_000, u64::MAX] {
            let a = retention_candidates_with_replica_floors_and_budget(
                &entries,
                16_384,
                128,
                &ordered_pins,
                &[16_385, floor],
                budget,
            )
            .unwrap();
            let b = retention_candidates_with_replica_floors_and_budget(
                &entries,
                16_384,
                128,
                &reversed_pins,
                &[floor, 16_385],
                budget,
            )
            .unwrap();
            assert_eq!(a, b, "floor={floor}, budget={budget}");
            assert!(a.0.iter().all(|&generation| {
                generation < floor
                    && generation < 16_384 - 128 + 1
                    && !ordered_pins.contains(&generation)
            }));
            assert!(a.1 <= budget);
            let actual_bytes =
                a.0.iter()
                    .map(|&id| entries[(id - 1) as usize].1)
                    .sum::<u64>();
            assert_eq!(a.1, actual_bytes);
        }
    }
}

#[test]
fn sorted_pin_validation_fails_closed_when_budget_is_zero() {
    let entries = (1..=128u64).map(|id| (id, 1)).collect::<Vec<_>>();
    assert!(
        retention_candidates_with_replica_floors_and_budget(
            &entries,
            128,
            1,
            &[1, 3, 5, 999, 1000],
            &[50],
            0
        )
        .is_err()
    );
    assert!(
        retention_candidates_with_replica_floors_and_budget(
            &entries,
            128,
            1,
            &[1, 3, 5, 5, 7],
            &[50],
            0
        )
        .is_err()
    );
}
