use super::*;

#[test]
fn large_ack_batches_validate_both_memberships_and_count_unique_votes() {
    let policy = QuorumPolicy::new(9, 5, 5).unwrap();
    let previous = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
    let next = ["b", "c", "d", "e", "f", "g", "h", "i", "j"];
    let acks = (0..128)
        .map(|index| if index % 2 == 0 { "a" } else { "j" })
        .collect::<Vec<_>>();
    let observed = observe_joint_quorum(policy, &previous, policy, &next, &acks).unwrap();
    assert_eq!(observed.previous.distinct_acks, 1);
    assert_eq!(observed.next.distinct_acks, 1);
    assert!(!observed.can_write);
    let mut invalid_old = previous;
    invalid_old[8] = "a";
    assert!(observe_joint_quorum(policy, &invalid_old, policy, &next, &acks).is_err());
    let mut invalid_new = next;
    invalid_new[8] = "b";
    assert!(observe_joint_quorum(policy, &previous, policy, &invalid_new, &acks).is_err());
    let mut bad_ack = acks;
    bad_ack[127] = "unknown";
    assert!(observe_joint_quorum(policy, &previous, policy, &next, &bad_ack).is_err());
}
