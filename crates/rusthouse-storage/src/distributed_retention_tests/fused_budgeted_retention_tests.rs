use super::*;

#[test]
fn fused_budget_matches_existing_pinned_eligibility_at_scale() {
    let generations = (1..=4096u64)
        .rev()
        .map(|id| (id, (id * 31) % 997))
        .collect::<Vec<_>>();
    let ids = generations.iter().map(|&(id, _)| id).collect::<Vec<_>>();
    let pinned = [3, 11, 121, 2048, 4095];
    let eligible = retention_candidates_with_pins(&ids, 4096, 100, &pinned).unwrap();
    let mut sizes = generations.clone();
    sizes.sort_unstable_by_key(|&(id, _)| id);
    for budget in [0u64, 10, 512, 20_000, u64::MAX] {
        let mut used = 0u64;
        let mut expected = Vec::new();
        for &id in &eligible {
            let bytes = sizes
                .binary_search_by_key(&id, |&(generation, _)| generation)
                .map(|index| sizes[index].1)
                .unwrap();
            if bytes <= budget - used {
                used += bytes;
                expected.push(id);
            }
        }
        assert_eq!(
            retention_candidates_with_byte_budget(&generations, 4096, 100, &pinned, budget,)
                .unwrap(),
            (expected, used),
        );
    }
}

#[test]
fn rejects_invalid_pins_even_when_every_generation_is_kept() {
    assert!(
        retention_candidates_with_byte_budget(&[(1, 4), (2, 8)], 2, 2, &[1, 1], 100,).is_err()
    );
    assert!(
        retention_candidates_with_byte_budget(&[(1, 4), (2, 8)], 2, 2, &[3], 100,).is_err()
    );
}
