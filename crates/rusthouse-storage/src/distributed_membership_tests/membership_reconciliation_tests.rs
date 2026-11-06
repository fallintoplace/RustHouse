use super::*;

#[test]
fn detects_add_remove_and_reconfiguration_in_canonical_order() {
    let before = parse_membership("c zone0 1\nb zone1 2\na zone1 3\n").unwrap();
    let after = parse_membership("d zone3 1\na zone1 3\nb zone2 2\n").unwrap();
    let changes = membership_changes(&before, &after).unwrap();
    assert_eq!(
        changes,
        vec![
            MembershipChange::Reconfigured {
                before: before[1].clone(),
                after: after[2].clone(),
            },
            MembershipChange::Removed(before[0].clone()),
            MembershipChange::Added(after[0].clone()),
        ]
    );
    let mut reordered = before.clone();
    reordered.reverse();
    assert!(membership_changes(&before, &reordered).unwrap().is_empty());
}

#[test]
fn large_manifest_only_clones_changed_members() {
    let before = (0..64)
        .map(|index| Member {
            node: format!("node_{index:03}"),
            zone: format!("zone_{}", index % 4),
            weight: 1,
        })
        .collect::<Vec<_>>();
    let mut after = before.clone();
    after.reverse();
    assert!(membership_changes(&before, &after).unwrap().is_empty());
    after[0].weight = 1000;
    after[63].zone = "new_zone".into();
    let changes = membership_changes(&before, &after).unwrap();
    assert_eq!(changes.len(), 2);
    assert!(matches!(changes[0], MembershipChange::Reconfigured { .. }));
    assert!(matches!(changes[1], MembershipChange::Reconfigured { .. }));
    let mut duplicate = before.clone();
    duplicate[1].node = duplicate[0].node.clone();
    assert!(membership_changes(&duplicate, &after).is_err());
    let mut invalid = after.clone();
    invalid[3].weight = 0;
    assert!(membership_changes(&before, &invalid).is_err());
}
