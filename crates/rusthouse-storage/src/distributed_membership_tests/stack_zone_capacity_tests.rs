use super::*;
use std::collections::BTreeMap;

#[test]
fn full_membership_matches_ordered_map_for_skewed_and_many_zone_inputs() {
    for zone_count in [1, 3, 17, 64] {
        let mut members = (0..64)
            .map(|index| Member {
                node: format!("node_{index:02}"),
                zone: format!("zone_{:02}", (index * 23) % zone_count),
                weight: (1 + (index * 7) % 1000) as u16,
            })
            .collect::<Vec<_>>();
        let mut oracle = BTreeMap::<String, (usize, u64)>::new();
        for member in &members {
            let value = oracle.entry(member.zone.clone()).or_default();
            value.0 += 1;
            value.1 += u64::from(member.weight);
        }
        let expected = oracle
            .into_iter()
            .map(|(zone, (nodes, weight))| ZoneCapacity {
                zone,
                nodes,
                weight,
            })
            .collect::<Vec<_>>();
        assert_eq!(membership_capacity_by_zone(&members).unwrap(), expected);
        members.reverse();
        assert_eq!(membership_capacity_by_zone(&members).unwrap(), expected);
    }
}
