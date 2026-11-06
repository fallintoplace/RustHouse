use super::*;
use crate::distributed_membership::parse_membership;

#[test]
fn large_zone_membership_stays_deterministic_when_reversed() {
    let input = (0..64)
        .map(|index| format!("node_{index} zone_{} {}\n", index % 8, 1 + index % 4))
        .collect::<String>();
    let members = parse_membership(&input).unwrap();
    let reversed = members.iter().rev().cloned().collect::<Vec<_>>();
    for shard in 0..16 {
        let name = format!("zone_test_{shard}");
        let expected = zone_aware_replicas_for(&name, &members, 8).unwrap();
        assert_eq!(
            expected,
            zone_aware_replicas_for(&name, &reversed, 8).unwrap()
        );
        assert_eq!(expected.len(), 8);
    }
}

#[test]
fn single_zone_fallback_matches_weighted_ranking_without_duplicates() {
    let membership = (0..64)
        .map(|index| format!("node_{index} same_zone {}\n", 1 + index % 4))
        .collect::<String>();
    let members = parse_membership(&membership).unwrap();
    let weighted = members
        .iter()
        .map(|member| (member.node.as_str(), member.weight))
        .collect::<Vec<_>>();
    let expected = weighted_replicas_for("fallback", &weighted, 64).unwrap();
    for replicas in [1, 2, 8, 32, 64] {
        assert_eq!(
            zone_aware_replicas_for("fallback", &members, replicas).unwrap(),
            expected[..replicas]
        );
    }
}

#[test]
fn places_two_replicas_in_different_zones() {
    let members = parse_membership("a east 7\nb east 3\nc west 1\n").unwrap();
    for index in 0..32 {
        let shard = format!("shard_{index}");
        let selected = zone_aware_replicas_for(&shard, &members, 2).unwrap();
        assert_eq!(selected.len(), 2);
        assert!(selected.contains(&"c".to_owned()));
        let reversed = members.iter().rev().cloned().collect::<Vec<_>>();
        assert_eq!(
            selected,
            zone_aware_replicas_for(&shard, &reversed, 2).unwrap()
        );
    }
    assert_eq!(zone_aware_replicas_for("s", &members, 3).unwrap().len(), 3);
    assert!(zone_aware_replicas_for("s", &members, 4).is_err());
}
