use super::*;

#[test]
fn oldest_eligible_candidates_skip_pins_and_newer_replica_history() {
    let generations = [
        (8, 10),
        (2, 60),
        (1, 20),
        (7, 2),
        (4, 4),
        (3, 30),
        (6, 5),
        (5, 8),
    ];
    assert_eq!(
        retention_candidates_with_replica_floors_and_budget(
            &generations,
            8,
            2,
            &[2],
            &[5, 7, 6],
            55,
        )
        .unwrap(),
        (vec![1, 3, 4], 54),
    );
    assert_eq!(
        retention_candidates_with_replica_floors_and_budget(
            &generations,
            8,
            2,
            &[2],
            &[1],
            u64::MAX,
        )
        .unwrap(),
        (vec![], 0),
    );
}

#[test]
fn combined_plan_matches_composed_reference_without_wasting_budget() {
    let inventory = (1..=1024u64)
        .rev()
        .map(|id| (id, (id * 31) % 67))
        .collect::<Vec<_>>();
    let ids = inventory.iter().map(|&(id, _)| id).collect::<Vec<_>>();
    let pinned = [4, 20, 509, 800];
    let sizes = inventory
        .iter()
        .copied()
        .collect::<std::collections::HashMap<_, _>>();
    for active in [1, 513, 1024] {
        for keep in [1usize, 5, 128, 1024, usize::MAX] {
            for floors in [&[1u64, 800][..], &[257, 1024][..], &[u64::MAX][..]] {
                let floor = *floors.iter().min().unwrap();
                let permitted = retention_candidates_with_pins(&ids, active, keep, &pinned)
                    .unwrap()
                    .into_iter()
                    .filter(|&id| id < floor)
                    .collect::<Vec<_>>();
                for budget in [0u64, 1, 15, 1024, 10000, u64::MAX] {
                    let mut bytes = 0u64;
                    let mut expected = Vec::new();
                    for &id in &permitted {
                        let size = sizes[&id];
                        if size <= budget - bytes {
                            bytes += size;
                            expected.push(id);
                        }
                    }
                    assert_eq!(
                        retention_candidates_with_replica_floors_and_budget(
                            &inventory, active, keep, &pinned, floors, budget,
                        )
                        .unwrap(),
                        (expected, bytes),
                    );
                }
            }
        }
    }
}

#[test]
fn invalid_replica_floors_or_pins_fail_even_under_zero_budget() {
    let inventory = [(1, 10), (2, 30), (3, 40)];
    for floors in [&[][..], &[0u64][..], &[1u64; 65][..]] {
        assert!(
            retention_candidates_with_replica_floors_and_budget(
                &inventory,
                3,
                1,
                &[],
                floors,
                0,
            )
            .is_err()
        );
    }
    for pins in [&[9u64][..], &[1u64, 1][..]] {
        assert!(
            retention_candidates_with_replica_floors_and_budget(
                &inventory,
                3,
                1,
                pins,
                &[2],
                0,
            )
            .is_err()
        );
    }
    assert!(
        retention_candidates_with_replica_floors_and_budget(
            &[(1, 10), (1, 20)],
            1,
            1,
            &[],
            &[2],
            100,
        )
        .is_err()
    );
}
