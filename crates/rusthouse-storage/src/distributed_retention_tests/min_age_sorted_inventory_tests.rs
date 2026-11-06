use super::*;

#[test]
fn chronological_and_shuffled_ages_match_exact_retention_policy() {
    let forward = (1..=4_096u64)
        .map(|generation| (generation, generation * 4))
        .collect::<Vec<_>>();
    let reversed = forward.iter().copied().rev().collect::<Vec<_>>();
    let planned = |inventory: &[(u64, u64)]| {
        retention_candidates_with_min_age(
            inventory,
            4_096,
            16,
            &[10, 200],
            &[3_500],
            16_384,
            800,
        )
        .unwrap()
    };
    assert_eq!(planned(&forward), planned(&reversed));
    assert!(
        planned(&forward)
            .iter()
            .all(|&generation| generation != 10 && generation != 200)
    );
    let mut duplicate = forward.clone();
    duplicate.push((4_096, 16_384));
    assert!(
        retention_candidates_with_min_age(&duplicate, 4_096, 2, &[], &[4_096], 16_384, 0)
            .is_err()
    );
    let mut future = forward;
    future[0].1 = 16_385;
    assert!(
        retention_candidates_with_min_age(&future, 4_096, 2, &[], &[4_096], 16_384, 0).is_err()
    );
}
