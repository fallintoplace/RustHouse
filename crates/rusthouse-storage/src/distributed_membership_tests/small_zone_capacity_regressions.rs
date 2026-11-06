use super::*;

fn membership(n: usize) -> Vec<Member> {
    (0..n)
        .map(|index| Member {
            node: format!("node{index}"),
            zone: format!("z{}", index % 3),
            weight: u16::try_from(index + 1).unwrap(),
        })
        .collect()
}

#[test]
fn small_and_large_membership_use_the_same_sorted_zone_totals() {
    for count in [1, 3, 8, 16, 17, 32, 64] {
        let members = membership(count);
        let zones = membership_capacity_by_zone(&members).unwrap();
        assert_eq!(zones.len(), count.min(3));
        for (index, zone) in zones.iter().enumerate() {
            assert_eq!(zone.zone, format!("z{index}"));
            let matching = members.iter().filter(|m| m.zone == zone.zone);
            assert_eq!(zone.nodes, matching.clone().count());
            assert_eq!(
                zone.weight,
                matching.map(|m| u64::from(m.weight)).sum::<u64>()
            );
        }
    }
}

#[test]
fn invalid_membership_is_rejected_before_aggregation() {
    let mut members = membership(3);
    members[2].node = members[0].node.clone();
    assert!(membership_capacity_by_zone(&members).is_err());
}
