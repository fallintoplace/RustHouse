use super::*;
use crate::distributed_membership::parse_membership;

#[test]
fn movement_sets_exactly_partition_old_and_new_replica_placements() {
    let old = parse_membership("a west 5\nb east 3\nc north 2\n").unwrap();
    let new = parse_membership("a west 5\nc north 2\nd south 8\ne east 1\n").unwrap();
    for replica_count in 1..=3 {
        for index in 0..128 {
            let shard = format!("shard_{index}");
            let previous = zone_aware_replicas_for(&shard, &old, replica_count).unwrap();
            let desired = zone_aware_replicas_for(&shard, &new, replica_count).unwrap();
            let plan = plan_zone_aware_rebalance(&shard, &old, &new, replica_count).unwrap();
            assert_eq!(plan.new_placement, desired);
            assert_eq!(
                plan.incoming.iter().chain(plan.retained.iter()).count(),
                replica_count
            );
            assert_eq!(plan.outgoing.len() + plan.retained.len(), replica_count);
            for node in &plan.incoming {
                assert!(desired.contains(node));
                assert!(!previous.contains(node));
            }
            for node in &plan.outgoing {
                assert!(previous.contains(node));
                assert!(!desired.contains(node));
            }
            for node in &plan.retained {
                assert!(previous.contains(node));
                assert!(desired.contains(node));
            }
            let reversed =
                plan_zone_aware_rebalance(&shard, &new, &old, replica_count).unwrap();
            let mut reversed_add = reversed.incoming.clone();
            let mut removed = plan.outgoing.clone();
            reversed_add.sort();
            removed.sort();
            assert_eq!(reversed_add, removed);
        }
    }
}

#[test]
fn unchanged_membership_needs_no_transfers_even_if_input_is_reversed() {
    let members = parse_membership("a west 2\nb east 3\nc north 4\n").unwrap();
    let reversed = members.iter().rev().cloned().collect::<Vec<_>>();
    for count in 1..=3 {
        let plan = plan_zone_aware_rebalance("stable", &members, &reversed, count).unwrap();
        assert!(plan.incoming.is_empty());
        assert!(plan.outgoing.is_empty());
        assert_eq!(plan.retained, plan.new_placement);
    }
    assert!(plan_zone_aware_rebalance("stable", &members, &[], 1).is_err());
    assert!(plan_zone_aware_rebalance("stable", &members, &members, 0).is_err());
}
