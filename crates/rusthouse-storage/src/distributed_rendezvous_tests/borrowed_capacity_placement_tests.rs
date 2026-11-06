use super::*;
use crate::distributed_capacity::Capacity;
use crate::distributed_membership::Member;

fn nodes() -> Vec<Member> {
    (0..24)
        .map(|index| Member {
            node: format!("node_{index}"),
            zone: format!("zone_{}", index % 4),
            weight: (index * 31 + 1) as u16,
        })
        .collect()
}

#[test]
fn entirely_eligible_membership_matches_direct_zone_ranking() {
    let members = nodes();
    let capacities = members
        .iter()
        .map(|member| {
            (
                member.node.as_str(),
                Capacity {
                    total: 200,
                    used: 50,
                    reserved: 25,
                },
            )
        })
        .collect::<Vec<_>>();
    for id in 0..128 {
        let shard = format!("shard_{id}");
        for replicas in [1, 2, 3, 8, 24] {
            assert_eq!(
                capacity_aware_replicas_for(&shard, &members, &capacities, 125, replicas),
                zone_aware_replicas_for(&shard, &members, replicas),
            );
        }
    }
}

#[test]
fn filtered_capacity_and_error_contracts_remain_unchanged() {
    let members = nodes();
    let capacities = members
        .iter()
        .enumerate()
        .map(|(index, member)| {
            (
                member.node.as_str(),
                Capacity {
                    total: 100,
                    used: if index % 3 == 0 { 99 } else { 0 },
                    reserved: 0,
                },
            )
        })
        .collect::<Vec<_>>();
    let eligible = members
        .iter()
        .enumerate()
        .filter_map(|(index, member)| (index % 3 != 0).then_some(member.clone()))
        .collect::<Vec<_>>();
    for id in 0..64 {
        let shard = format!("shard_{id}");
        for replicas in [1, 2, 4, 16] {
            assert_eq!(
                capacity_aware_replicas_for(&shard, &members, &capacities, 2, replicas),
                zone_aware_replicas_for(&shard, &eligible, replicas),
            );
        }
    }
    assert!(capacity_aware_replicas_for("shard", &members, &capacities, 2, 17).is_err());
    let mut broken = capacities.clone();
    broken[20].1.reserved = u64::MAX;
    assert!(capacity_aware_replicas_for("shard", &members, &broken, 2, 1).is_err());
}
