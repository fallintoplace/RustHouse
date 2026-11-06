use super::*;

#[test]
fn small_and_large_candidate_sets_preserve_stable_ranking() {
    for count in [1usize, 2, 3, 8, 9, 16, 64] {
        let ids = (0..count)
            .map(|i| format!("node_{i:02}"))
            .collect::<Vec<_>>();
        let weighted = ids
            .iter()
            .enumerate()
            .map(|(i, name)| (name.as_str(), 1 + (i % 17) as u16))
            .collect::<Vec<_>>();
        for requested in [1, count] {
            let mut reverse_ids = ids.clone();
            reverse_ids.reverse();
            let mut reverse_weights = weighted.clone();
            reverse_weights.reverse();
            assert_eq!(
                replicas_for("shard_a", &ids, requested).unwrap(),
                replicas_for("shard_a", &reverse_ids, requested).unwrap(),
            );
            assert_eq!(
                weighted_replicas_for("shard_a", &weighted, requested).unwrap(),
                weighted_replicas_for("shard_a", &reverse_weights, requested).unwrap(),
            );
        }
        if count > 1 {
            let mut duplicate_ids = ids.clone();
            duplicate_ids[1] = duplicate_ids[0].clone();
            assert!(replicas_for("shard_a", &duplicate_ids, 1).is_err());
            let mut duplicate_weights = weighted.clone();
            duplicate_weights[1].0 = duplicate_weights[0].0;
            assert!(weighted_replicas_for("shard_a", &duplicate_weights, 1).is_err());
        }
    }
}
