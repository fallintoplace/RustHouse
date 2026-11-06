use super::*;

#[test]
fn full_width_membership_and_joint_memberships_count_distinct_votes() {
    let old = (0..64).map(|i| format!("old{i}")).collect::<Vec<_>>();
    let new = (0..64).map(|i| format!("new{i}")).collect::<Vec<_>>();
    let a = old.iter().map(String::as_str).collect::<Vec<_>>();
    let b = new.iter().map(String::as_str).collect::<Vec<_>>();
    let policy = QuorumPolicy::new(64, 33, 33).unwrap();
    assert_eq!(policy.observe(&a, &a).unwrap().distinct_acks, 64);
    assert_eq!(
        policy.observe(&a, &[a[63], a[63]]).unwrap().distinct_acks,
        1
    );
    let observations = a.iter().chain(b.iter()).copied().collect::<Vec<_>>();
    let joint = observe_joint_quorum(policy, &a, policy, &b, &observations).unwrap();
    assert_eq!(joint.previous.distinct_acks, 64);
    assert_eq!(joint.next.distinct_acks, 64);
    assert!(joint.can_read && joint.can_write);
    assert!(policy.observe(&a, &["unknown"]).is_err());
}

#[test]
fn joint_recovery_can_select_across_more_than_32_unique_members() {
    let old = (0..40).map(|i| format!("old{i}")).collect::<Vec<_>>();
    let new = (0..40).map(|i| format!("new{i}")).collect::<Vec<_>>();
    let a = old.iter().map(String::as_str).collect::<Vec<_>>();
    let b = new.iter().map(String::as_str).collect::<Vec<_>>();
    let policy = QuorumPolicy::new(40, 21, 21).unwrap();
    let selected = plan_joint_write_acknowledgements(policy, &a, policy, &b, &[]).unwrap();
    assert_eq!(selected.len(), 42);
    let references = selected.iter().map(String::as_str).collect::<Vec<_>>();
    assert!(
        observe_joint_quorum(policy, &a, policy, &b, &references)
            .unwrap()
            .can_write
    );
}
