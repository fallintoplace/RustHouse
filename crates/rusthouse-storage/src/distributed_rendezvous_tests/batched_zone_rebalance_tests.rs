use super::*;
use crate::distributed_membership::parse_membership;

#[test]
fn duplicate_and_distinct_shards_match_individual_rebalance_plans() {
    let before =
        parse_membership("a west 5\nb east 3\nc north 2\nd south 4\ne west 1\n").unwrap();
    let after =
        parse_membership("a west 5\nc north 2\nd south 4\ne west 1\nf east 3\n").unwrap();
    for replicas in [1usize, 2, 3, 5] {
        for count in [0usize, 1, 16, 17, 257, 4096] {
            let names = (0..count)
                .map(|index| format!("shard-{:02}", index % 19))
                .collect::<Vec<_>>();
            let shards = names.iter().map(String::as_str).collect::<Vec<_>>();
            let plans =
                plan_zone_aware_rebalance_many(&shards, &before, &after, replicas).unwrap();
            assert_eq!(plans.len(), shards.len());
            for (shard, actual) in shards.iter().zip(&plans) {
                assert_eq!(
                    *actual,
                    plan_zone_aware_rebalance(shard, &before, &after, replicas).unwrap(),
                    "replicas={replicas}, shard={shard}",
                );
            }
            if count > 19 {
                assert_eq!(plans[0], plans[19]);
            }
        }
    }
}

#[test]
fn validates_memberships_and_every_shard_even_for_empty_or_late_inputs() {
    let before = parse_membership("a west 5\nb east 3\n").unwrap();
    let after = parse_membership("a west 5\nc north 2\n").unwrap();
    assert!(
        plan_zone_aware_rebalance_many(&[], &before, &after, 1)
            .unwrap()
            .is_empty()
    );
    assert!(plan_zone_aware_rebalance_many(&[], &before, &after, 0).is_err());
    assert!(plan_zone_aware_rebalance_many(&[], &before, &after, 3).is_err());
    assert!(plan_zone_aware_rebalance_many(&[], &[], &after, 1).is_err());
    let mut shards = vec!["valid"; 4095];
    shards.push("../invalid");
    assert!(plan_zone_aware_rebalance_many(&shards, &before, &after, 2).is_err());
    assert!(plan_zone_aware_rebalance_many(&vec!["valid"; 4097], &before, &after, 2).is_err());
}

#[test]
fn membership_order_does_not_change_batched_placements() {
    let mut before = parse_membership("a west 5\nb east 3\nc south 2\n").unwrap();
    let mut after = parse_membership("a west 5\nc south 2\nd north 7\n").unwrap();
    let shards = [
        "one", "two", "one", "three", "two", "four", "five", "six", "seven", "eight", "nine",
        "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen",
    ];
    let expected = plan_zone_aware_rebalance_many(&shards, &before, &after, 2).unwrap();
    before.reverse();
    after.reverse();
    assert_eq!(
        plan_zone_aware_rebalance_many(&shards, &before, &after, 2).unwrap(),
        expected,
    );
}
