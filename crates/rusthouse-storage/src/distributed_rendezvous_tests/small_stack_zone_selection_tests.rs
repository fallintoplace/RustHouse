use super::*;
use crate::distributed_membership::parse_membership;

#[test]
fn small_zone_rankings_match_weighted_reference_for_all_prefix_lengths() {
    for count in [3, 4, 6, 8, 9, 16] {
        let manifest = (0..count)
            .map(|i| format!("n{i} zone{} {}\n", i % 3, i + 1))
            .collect::<String>();
        let members = parse_membership(&manifest).unwrap();
        let weights = members
            .iter()
            .map(|m| (m.node.as_str(), m.weight))
            .collect::<Vec<_>>();
        for shard in 0..12 {
            let shard = format!("small_{shard}");
            let ranking = weighted_replicas_for(&shard, &weights, count).unwrap();
            let mut seen = HashSet::new();
            let mut first = Vec::new();
            let mut fallback = Vec::new();
            for node in ranking {
                let zone = &members.iter().find(|m| m.node == node).unwrap().zone;
                if seen.insert(zone) {
                    first.push(node);
                } else {
                    fallback.push(node);
                }
            }
            first.extend(fallback);
            for replicas in 3..=count {
                assert_eq!(
                    zone_aware_replicas_for(&shard, &members, replicas).unwrap(),
                    first[..replicas],
                    "count={count}, replicas={replicas}"
                );
            }
        }
    }
}
