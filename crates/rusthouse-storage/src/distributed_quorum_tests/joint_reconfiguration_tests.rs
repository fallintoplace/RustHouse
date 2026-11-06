use super::*;

#[test]
fn requires_both_old_and_new_acks_throughout_membership_change() {
    let policy = QuorumPolicy::new(3, 2, 2).unwrap();
    let old = ["a", "b", "c"];
    let new = ["b", "c", "d"];
    let old_only = observe_joint_quorum(policy, &old, policy, &new, &["a", "b"]).unwrap();
    assert_eq!(old_only.previous.distinct_acks, 2);
    assert_eq!(old_only.next.distinct_acks, 1);
    assert!(!old_only.can_write);
    let new_only = observe_joint_quorum(policy, &old, policy, &new, &["c", "d"]).unwrap();
    assert_eq!(new_only.previous.distinct_acks, 1);
    assert_eq!(new_only.next.distinct_acks, 2);
    assert!(!new_only.can_write);
    let both = observe_joint_quorum(policy, &old, policy, &new, &["a", "b", "d", "b"]).unwrap();
    assert!(both.can_read && both.can_write);
    assert_eq!(both.previous.distinct_acks, 2);
    assert_eq!(both.next.distinct_acks, 2);
    assert!(observe_joint_quorum(policy, &old, policy, &new, &["unknown"]).is_err());
}

#[test]
fn validates_both_memberships_and_quorum_policies() {
    let policy = QuorumPolicy::new(3, 2, 2).unwrap();
    assert!(
        observe_joint_quorum(policy, &["a", "a", "c"], policy, &["b", "c", "d"], &[]).is_err()
    );
    assert!(
        observe_joint_quorum(policy, &["a", "b", "c"], policy, &["b", "b", "d"], &[]).is_err()
    );
    let unsafe_policy = QuorumPolicy {
        replicas: 3,
        reads: 1,
        writes: 1,
    };
    assert!(
        observe_joint_quorum(
            unsafe_policy,
            &["a", "b", "c"],
            policy,
            &["b", "c", "d"],
            &[]
        )
        .is_err()
    );
    let votes = vec!["b"; 257];
    assert!(
        observe_joint_quorum(policy, &["a", "b", "c"], policy, &["b", "c", "d"], &votes)
            .is_err()
    );
}

#[test]
fn counts_shared_members_once_in_each_large_configuration() {
    let old_ids = (0..16).map(|i| format!("n{i}")).collect::<Vec<_>>();
    let new_ids = (8..24).map(|i| format!("n{i}")).collect::<Vec<_>>();
    let old = old_ids.iter().map(String::as_str).collect::<Vec<_>>();
    let new = new_ids.iter().map(String::as_str).collect::<Vec<_>>();
    let policy = QuorumPolicy::new(16, 9, 9).unwrap();
    let acknowledgements = (0..16)
        .map(|i| format!("n{i}"))
        .chain((16..24).map(|i| format!("n{i}")))
        .collect::<Vec<_>>();
    let votes = acknowledgements
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let result = observe_joint_quorum(policy, &old, policy, &new, &votes).unwrap();
    assert_eq!(result.previous.distinct_acks, 16);
    assert_eq!(result.next.distinct_acks, 16);
    assert!(result.can_write);
}
