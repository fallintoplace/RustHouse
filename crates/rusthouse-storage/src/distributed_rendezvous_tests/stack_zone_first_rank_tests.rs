use super::*;
use crate::distributed_membership::Member;

#[test]
fn stack_zone_ids_match_independent_weighted_zone_first_order() {
    for zone_count in [1, 3, 17, 64] {
        let mut members = (0..64usize)
            .map(|index| Member {
                node: format!("replica_{index:02}"),
                zone: format!("zone_{:02}", (index * 17) % zone_count),
                weight: (1 + index * 13) as u16,
            })
            .collect::<Vec<_>>();
        for reversed in [false, true] {
            if reversed {
                members.reverse();
            }
            let nodes = members
                .iter()
                .map(|node| (node.node.as_str(), node.weight))
                .collect::<Vec<_>>();
            for shard in ["events", "logs", "shard_2048"] {
                let weighted = weighted_replicas_for(shard, &nodes, 64).unwrap();
                let mut seen = HashSet::new();
                let mut first = Vec::new();
                let mut remaining = Vec::new();
                for name in weighted {
                    let zone = &members
                        .iter()
                        .find(|member| member.node == name)
                        .unwrap()
                        .zone;
                    if seen.insert(zone.as_str()) {
                        first.push(name);
                    } else {
                        remaining.push(name);
                    }
                }
                first.extend(remaining);
                for replicas in [3, 4, 16, 32, 64] {
                    assert_eq!(
                        zone_aware_replicas_for(shard, &members, replicas).unwrap(),
                        first[..replicas],
                        "zones={zone_count} reversed={reversed} replicas={replicas}"
                    );
                }
            }
        }
    }
}
