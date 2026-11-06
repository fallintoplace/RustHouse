use super::*;

#[test]
fn reports_stable_totals_by_failure_domain() {
    let members = parse_membership("c zone_b 2\na zone_a 4\nb zone_a 3\n").unwrap();
    assert_eq!(
        membership_capacity_by_zone(&members).unwrap(),
        vec![
            ZoneCapacity {
                zone: "zone_a".into(),
                nodes: 2,
                weight: 7
            },
            ZoneCapacity {
                zone: "zone_b".into(),
                nodes: 1,
                weight: 2
            },
        ]
    );
    assert!(membership_capacity_by_zone(&[]).is_err());
    assert!(
        membership_capacity_by_zone(&[Member {
            node: "bad".into(),
            zone: "a".into(),
            weight: 0
        }])
        .is_err()
    );
}
