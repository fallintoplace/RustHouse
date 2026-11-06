use super::*;

#[test]
fn many_repeated_votes_match_independent_old_and_new_observations() {
    let previous = QuorumPolicy::new(16, 8, 9).unwrap();
    let next = QuorumPolicy::new(16, 8, 9).unwrap();
    let old = (0..16).map(|i| format!("old{i}")).collect::<Vec<_>>();
    let new = (8..24).map(|i| format!("old{i}")).collect::<Vec<_>>();
    let old_names = old.iter().map(String::as_str).collect::<Vec<_>>();
    let new_names = new.iter().map(String::as_str).collect::<Vec<_>>();
    let votes = (0..240)
        .map(|i| {
            if i % 2 == 0 {
                old_names[(i / 2) % 16]
            } else {
                new_names[(i / 2) % 16]
            }
        })
        .collect::<Vec<_>>();
    let observed =
        observe_joint_quorum(previous, &old_names, next, &new_names, &votes).unwrap();
    let old_votes = votes
        .iter()
        .copied()
        .filter(|v| old_names.contains(v))
        .collect::<Vec<_>>();
    let new_votes = votes
        .iter()
        .copied()
        .filter(|v| new_names.contains(v))
        .collect::<Vec<_>>();
    assert_eq!(
        observed.previous,
        previous.observe(&old_names, &old_votes).unwrap()
    );
    assert_eq!(observed.next, next.observe(&new_names, &new_votes).unwrap());
    assert!(observed.can_write);
    assert!(observed.can_read);
}

#[test]
fn indexed_validation_rejects_duplicate_members_and_unknown_votes() {
    let policy = QuorumPolicy::new(3, 2, 2).unwrap();
    let votes = vec!["a"; 33];
    assert!(
        observe_joint_quorum(policy, &["a", "a", "b"], policy, &["a", "b", "c"], &votes,)
            .is_err()
    );
    assert!(
        observe_joint_quorum(
            policy,
            &["a", "b", "c"],
            policy,
            &["a", "b", "c"],
            &vec!["unknown"; 33],
        )
        .is_err()
    );
}
