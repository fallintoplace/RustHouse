use super::*;
use crate::distributed_membership::parse_membership;

#[test]
fn borrowed_rank_matches_full_weighted_order_across_replica_counts() {
    let manifest = (0..64)
        .map(|i| format!("n_{i} z_{} {}\n", i % 7, (i % 100) + 1))
        .collect::<String>();
    let members = parse_membership(&manifest).unwrap();
    let weights = members
        .iter()
        .map(|member| (member.node.as_str(), member.weight))
        .collect::<Vec<_>>();
    for shard in 0..40 {
        let shard = format!("events_{shard}");
        let rank = weighted_replicas_for(&shard, &weights, members.len()).unwrap();
        let mut zones = HashSet::new();
        let mut expected = Vec::new();
        let mut fallback = Vec::new();
        for name in rank {
            let member = members.iter().find(|member| member.node == name).unwrap();
            if zones.insert(member.zone.as_str()) {
                expected.push(name);
            } else {
                fallback.push(name);
            }
        }
        expected.extend(fallback);
        for replicas in [2, 3, 7, 8, 16, 64] {
            assert_eq!(
                zone_aware_replicas_for(&shard, &members, replicas).unwrap(),
                expected[..replicas],
                "shard={shard}, replicas={replicas}",
            );
        }
    }
}
