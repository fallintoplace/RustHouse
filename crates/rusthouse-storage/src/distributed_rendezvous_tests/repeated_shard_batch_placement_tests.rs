use super::*;

#[test]
fn stack_ranking_matches_full_order_for_three_to_sixty_four_replicas() {
    let nodes = (0..64)
        .map(|number| format!("replica_{number:02}"))
        .collect::<Vec<_>>();
    for replicas in [3usize, 4, 7, 31, 63, 64] {
        let seed = shard_seed("part_42");
        let mut expected = nodes
            .iter()
            .map(|node| (score_with_seed(seed, node), node.as_str()))
            .collect::<Vec<_>>();
        expected.sort_unstable_by(|left, right| {
            right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1))
        });
        let expected = expected
            .into_iter()
            .take(replicas)
            .map(|(_, node)| node.to_owned())
            .collect::<Vec<_>>();
        assert_eq!(replicas_for("part_42", &nodes, replicas).unwrap(), expected);
    }
}

#[test]
fn repeated_large_batches_match_independent_rendezvous_placements() {
    let nodes = (0..64)
        .map(|number| format!("node_{number:02}"))
        .collect::<Vec<_>>();
    let names = (0..19)
        .map(|number| format!("shard_{number:02}"))
        .collect::<Vec<_>>();
    let shards = (0..4_096)
        .map(|index| names[(index * 37) % names.len()].as_str())
        .collect::<Vec<_>>();
    for replicas in [1, 2, 3, 8] {
        let batched = replicas_for_many(&shards, &nodes, replicas).unwrap();
        let independently = shards
            .iter()
            .map(|&shard| replicas_for(shard, &nodes, replicas).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(batched, independently, "replicas={replicas}");
    }
    assert!(replicas_for_many(&["shard_0", "invalid/path"], &nodes, 1).is_err());
}
