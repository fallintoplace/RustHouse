use super::*;
use crate::distributed_membership::parse_membership;

#[test]
fn mixed_small_and_large_placements_preserve_transfer_sets() {
    let old = parse_membership(
        "a z0 1\nb z1 2\nc z2 3\nd z3 4\ne z4 5\nf z5 6\ng z6 7\nh z7 8\ni z8 9\n",
    )
    .unwrap();
    let new = parse_membership(
        "a z0 1\nc z2 3\ne z4 5\ng z6 7\ni z8 9\nj z9 10\nk z10 11\nl z11 12\nm z12 13\n",
    )
    .unwrap();
    for replicas in [1, 3, 8, 9] {
        for shard in ["first", "second", "third"] {
            let plan = plan_zone_aware_rebalance(shard, &old, &new, replicas).unwrap();
            let previous = zone_aware_replicas_for(shard, &old, replicas).unwrap();
            let desired = zone_aware_replicas_for(shard, &new, replicas).unwrap();
            assert_eq!(plan.new_placement, desired);
            assert_eq!(
                plan.incoming,
                desired
                    .iter()
                    .filter(|node| !previous.contains(node))
                    .cloned()
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                plan.retained,
                desired
                    .iter()
                    .filter(|node| previous.contains(node))
                    .cloned()
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                plan.outgoing,
                previous
                    .iter()
                    .filter(|node| !desired.contains(node))
                    .cloned()
                    .collect::<Vec<_>>()
            );
        }
    }
}
