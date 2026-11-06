use super::*;
use crate::distributed_membership::parse_membership;

#[test]
fn recovery_chooses_minimum_number_of_lost_zones() {
    let members = parse_membership(
        "a east 1\nb east 1\nc east 1\nd west 1\ne west 1\nf north 1\ng south 1\n",
    )
    .unwrap();
    let policy = QuorumPolicy::new(7, 4, 4).unwrap();
    let zones = ["east", "west", "north", "south"];
    for mask in 0usize..16 {
        let down = zones
            .iter()
            .enumerate()
            .filter_map(|(i, zone)| ((mask >> i) & 1 != 0).then_some(*zone))
            .collect::<Vec<_>>();
        let plan = plan_zone_quorum_recovery(policy, &members, &down).unwrap();
        let surviving = members
            .iter()
            .filter(|m| !down.contains(&m.zone.as_str()))
            .count();
        assert_eq!(plan.surviving_replicas, surviving);
        for (selected, threshold) in [
            (&plan.restore_for_reads, policy.reads),
            (&plan.restore_for_writes, policy.writes),
        ] {
            let regained = selected
                .iter()
                .map(|zone| members.iter().filter(|m| m.zone == *zone).count())
                .sum::<usize>();
            assert!(surviving + regained >= threshold);
            let mut min_count = usize::MAX;
            for subset in 0usize..(1usize << down.len()) {
                let added = down
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| subset & (1usize << *i) != 0)
                    .map(|(_, zone)| members.iter().filter(|m| m.zone == *zone).count())
                    .sum::<usize>();
                if surviving + added >= threshold {
                    min_count = min_count.min(subset.count_ones() as usize);
                }
            }
            assert_eq!(selected.len(), min_count);
        }
    }
}

#[test]
fn unknown_duplicate_and_mismatched_outages_are_rejected() {
    let members = parse_membership("a east 1\nb west 1\nc south 1\n").unwrap();
    let policy = QuorumPolicy::new(3, 2, 2).unwrap();
    let result = plan_zone_quorum_recovery(policy, &members, &["east", "west"]);
    assert_eq!(result.unwrap().restore_for_reads, vec!["east".to_owned()]);
    assert!(plan_zone_quorum_recovery(policy, &members, &["unknown"]).is_err());
    assert!(plan_zone_quorum_recovery(policy, &members, &["east", "east"]).is_err());
    assert!(
        plan_zone_quorum_recovery(QuorumPolicy::new(4, 3, 3).unwrap(), &members, &[],).is_err()
    );
}
