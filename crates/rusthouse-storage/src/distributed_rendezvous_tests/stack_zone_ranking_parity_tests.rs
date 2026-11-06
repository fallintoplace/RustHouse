use super::*;
use crate::distributed_membership::Member;

#[test]
fn zone_first_order_matches_weighted_reference_across_boundaries() {
    let members = (0..64)
        .map(|index| Member {
            node: format!("replica_{index:02}"),
            zone: format!("zone_{}", index % 11),
            weight: (index % 17 + 1) as u16,
        })
        .collect::<Vec<_>>();
    let weighted = members
        .iter()
        .map(|member| (member.node.as_str(), member.weight))
        .collect::<Vec<_>>();
    for shard in ["alpha", "beta", "gamma"] {
        let ranked = weighted_replicas_for(shard, &weighted, members.len()).unwrap();
        let mut seen = HashSet::new();
        let mut first = Vec::new();
        let mut later = Vec::new();
        for node in ranked {
            let zone = &members
                .iter()
                .find(|member| member.node == node)
                .unwrap()
                .zone;
            if seen.insert(zone.as_str()) {
                first.push(node);
            } else {
                later.push(node);
            }
        }
        first.extend(later);
        for count in [3, 4, 8, 11, 12, 32, 64] {
            assert_eq!(
                zone_aware_replicas_for(shard, &members, count).unwrap(),
                first[..count]
            );
        }
    }
}
