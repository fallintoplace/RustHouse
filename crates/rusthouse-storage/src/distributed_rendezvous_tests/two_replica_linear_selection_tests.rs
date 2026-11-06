use super::*;
use crate::distributed_membership::Member;

#[test]
fn linear_path_matches_full_ranked_zone_aware_reference() {
    for zones in [1usize, 2, 3, 8, 64] {
        let members = (0..64)
            .map(|index| Member {
                node: format!("node_{index:02}"),
                zone: format!("zone_{:02}", index % zones),
                weight: ((index * 31) % 1000 + 1) as u16,
            })
            .collect::<Vec<_>>();
        for shard in ["alpha", "beta", "gamma", "delta", "shard_100"] {
            let seed = shard_seed(shard);
            let mut ranked = members
                .iter()
                .map(|member| {
                    let priority =
                        -hash_to_open_unit_interval(score_with_seed(seed, &member.node)).ln()
                            / f64::from(member.weight);
                    (priority, member)
                })
                .collect::<Vec<_>>();
            ranked.sort_unstable_by(|left, right| {
                left.0
                    .total_cmp(&right.0)
                    .then_with(|| left.1.node.cmp(&right.1.node))
            });
            let first = ranked[0].1;
            let second = ranked
                .iter()
                .find(|(_, member)| member.zone != first.zone)
                .map_or_else(|| ranked[1].1, |(_, member)| *member);
            let expected = vec![first.node.clone(), second.node.clone()];
            assert_eq!(
                zone_aware_replicas_for(shard, &members, 2).unwrap(),
                expected
            );
            let mut reversed = members.clone();
            reversed.reverse();
            assert_eq!(
                zone_aware_replicas_for(shard, &reversed, 2).unwrap(),
                expected
            );
        }
    }
}
