use super::*;

#[test]
fn sorted_and_shuffled_pins_produce_the_same_retention_plans() {
    let generations = (1..=4096u64).rev().collect::<Vec<_>>();
    let sizes = generations
        .iter()
        .map(|&id| (id, (id * 23) % 991))
        .collect::<Vec<_>>();
    for pins in [
        vec![2, 3, 11, 1024, 2048],
        vec![1, 128, 256, 512, 1024, 2048, 4095],
        (1..=4096).filter(|i| i % 5 == 0).collect::<Vec<_>>(),
    ] {
        let mut reversed = pins.clone();
        reversed.reverse();
        for keep in [1usize, 128, 4096] {
            assert_eq!(
                retention_candidates_with_pins(&generations, 4096, keep, &pins).unwrap(),
                retention_candidates_with_pins(&generations, 4096, keep, &reversed).unwrap(),
            );
            for budget in [0u64, 200, 10_000, u64::MAX] {
                assert_eq!(
                    retention_candidates_with_byte_budget(&sizes, 4096, keep, &pins, budget,)
                        .unwrap(),
                    retention_candidates_with_byte_budget(
                        &sizes, 4096, keep, &reversed, budget,
                    )
                    .unwrap(),
                );
            }
        }
    }
}

#[test]
fn sorted_pin_fast_path_never_accepts_unknown_or_duplicate_ids() {
    let generations = (1..=32u64).collect::<Vec<_>>();
    let sizes = generations.iter().map(|&id| (id, 1u64)).collect::<Vec<_>>();
    for pins in [vec![1, 2, 2, 3, 4, 5], vec![1, 2, 3, 4, 100]] {
        assert!(retention_candidates_with_pins(&generations, 32, 2, &pins,).is_err());
        assert!(retention_candidates_with_byte_budget(&sizes, 32, 2, &pins, 100,).is_err());
    }
}
