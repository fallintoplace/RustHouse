use super::*;

fn member(node: &str, zone: &str, weight: u16) -> Member {
    Member {
        node: node.into(),
        zone: zone.into(),
        weight,
    }
}

#[test]
fn generated_deltas_replay_exactly_in_canonical_order() {
    for count in [1usize, 2, 8, 16, 32, 64] {
        let before = (0..count)
            .map(|i| member(&format!("n{i:03}"), "z1", 1))
            .collect::<Vec<_>>();
        let mut after = before
            .iter()
            .enumerate()
            .filter(|&(i, _)| i % 4 != 0)
            .map(|(i, m)| {
                let mut next = m.clone();
                if i % 3 == 0 {
                    next.weight = 2;
                }
                next
            })
            .collect::<Vec<_>>();
        after.push(member("replacement", "z2", 3));
        let changes = membership_changes(&before, &after).unwrap();
        let actual = apply_membership_changes(&before, &changes).unwrap();
        after.sort_unstable_by(|a, b| a.node.cmp(&b.node));
        assert_eq!(actual, after, "count={count}");
        assert_eq!(apply_membership_changes(&actual, &[]).unwrap(), actual);
    }
}

#[test]
fn stale_duplicate_and_renaming_changes_fail_closed() {
    let before = vec![member("a", "east", 1), member("b", "west", 1)];
    let remove = MembershipChange::Removed(member("a", "east", 1));
    assert!(apply_membership_changes(&before, &[remove.clone(), remove]).is_err());
    assert!(
        apply_membership_changes(
            &before,
            &[MembershipChange::Removed(member("a", "east", 2))]
        )
        .is_err()
    );
    assert!(
        apply_membership_changes(&before, &[MembershipChange::Added(member("a", "west", 1))])
            .is_err()
    );
    assert!(
        apply_membership_changes(
            &before,
            &[MembershipChange::Reconfigured {
                before: member("a", "east", 1),
                after: member("renamed", "east", 1),
            }]
        )
        .is_err()
    );
    assert!(
        apply_membership_changes(
            &before,
            &[
                MembershipChange::Removed(member("a", "east", 1)),
                MembershipChange::Removed(member("b", "west", 1)),
            ],
        )
        .is_err()
    );
    assert!(
        apply_membership_changes(
            &before,
            &vec![MembershipChange::Added(member("x", "east", 1)); 129]
        )
        .is_err()
    );
    assert_eq!(before[0].node, "a");
}
