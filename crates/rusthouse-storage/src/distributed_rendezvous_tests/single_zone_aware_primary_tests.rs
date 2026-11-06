use super::*;
use crate::distributed_membership::parse_membership;

#[test]
fn primary_matches_weighted_selection_across_zones_and_member_orders() {
    let members = parse_membership("a east 1\nb west 8\nc east 4\nd north 12\n").unwrap();
    let weights = members
        .iter()
        .map(|member| (member.node.as_str(), member.weight))
        .collect::<Vec<_>>();
    let reversed = members.iter().rev().cloned().collect::<Vec<_>>();
    for index in 0..256 {
        let shard = format!("shard_{index}");
        let best = weighted_replicas_for(&shard, &weights, 1).unwrap();
        assert_eq!(zone_aware_replicas_for(&shard, &members, 1).unwrap(), best);
        assert_eq!(zone_aware_replicas_for(&shard, &reversed, 1).unwrap(), best);
    }
    assert!(zone_aware_replicas_for("shard", &members, 0).is_err());
}
