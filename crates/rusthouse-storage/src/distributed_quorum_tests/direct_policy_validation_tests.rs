use super::*;

#[test]
fn publicly_constructed_invalid_policy_cannot_report_quorum() {
    let unsafe_policy = QuorumPolicy {
        replicas: 3,
        reads: 0,
        writes: 1,
    };
    assert!(unsafe_policy.observe(&["a", "b", "c"], &["a"]).is_err());
    assert!(!unsafe_policy.can_read(3));
    assert!(!unsafe_policy.can_write(3));
}
