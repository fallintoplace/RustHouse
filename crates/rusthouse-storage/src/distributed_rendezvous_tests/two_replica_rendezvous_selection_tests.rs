use super::*;

#[test]
fn two_replicas_match_full_sort_for_tiny_and_large_candidate_sets() {
    for count in [2usize, 3, 8, 9, 16, 64] {
        let nodes = (0..count)
            .map(|index| format!("node_{index:02}"))
            .collect::<Vec<_>>();
        for index in 0..64 {
            let shard = format!("placement_{index}");
            let seed = shard_seed(&shard);
            let mut ranked = nodes
                .iter()
                .map(|node| (score_with_seed(seed, node), node.as_str()))
                .collect::<Vec<_>>();
            ranked.sort_unstable_by(|left, right| {
                right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1))
            });
            let expected = ranked
                .iter()
                .take(2)
                .map(|(_, node)| node.to_string())
                .collect::<Vec<_>>();
            assert_eq!(replicas_for(&shard, &nodes, 2).unwrap(), expected);
            let reversed = nodes.iter().rev().cloned().collect::<Vec<_>>();
            assert_eq!(replicas_for(&shard, &reversed, 2).unwrap(), expected);
        }
    }
}
