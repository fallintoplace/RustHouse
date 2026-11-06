use super::*;

#[test]
fn unsorted_snapshot_diffs_keep_canonical_order_and_values() {
    let before = parse_membership("gamma z1 1\nalpha z1 1\nbeta z2 2\n").unwrap();
    let after = parse_membership("beta z2 3\ndelta z1 1\nalpha z1 1\n").unwrap();
    let changes = membership_changes(&before, &after).unwrap();
    assert_eq!(
        changes,
        vec![
            MembershipChange::Reconfigured {
                before: before[2].clone(),
                after: after[0].clone(),
            },
            MembershipChange::Added(after[1].clone()),
            MembershipChange::Removed(before[0].clone()),
        ]
    );

    let mut reversed_before = before.clone();
    reversed_before.reverse();
    let mut reversed_after = after.clone();
    reversed_after.reverse();
    assert_eq!(
        membership_changes(&reversed_before, &reversed_after).unwrap(),
        changes
    );
}

#[test]
fn fully_identical_maximum_memberships_do_not_emit_changes() {
    let nodes = (0..64)
        .map(|index| Member {
            node: format!("node_{index:02}"),
            zone: format!("zone_{}", index % 8),
            weight: 1,
        })
        .collect::<Vec<_>>();
    let mut reverse = nodes.clone();
    reverse.reverse();
    assert!(membership_changes(&nodes, &reverse).unwrap().is_empty());
}
