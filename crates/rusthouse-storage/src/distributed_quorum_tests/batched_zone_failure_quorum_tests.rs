use super::*;
use crate::distributed_membership::parse_membership;

#[test]
fn one_pass_failure_curve_matches_independent_worst_case_queries() {
    let members =
        parse_membership("a east 1\nb east 1\nc west 1\nd south 1\ne south 1\nf north 1\n")
            .unwrap();
    let policy = QuorumPolicy::new(6, 3, 4).unwrap();
    let outcomes = zone_failure_quorum_curve(policy, &members).unwrap();
    assert_eq!(outcomes.len(), 5);
    for (failed, outcome) in outcomes.iter().enumerate() {
        assert_eq!(
            *outcome,
            worst_case_zone_quorum(policy, &members, failed).unwrap()
        );
        assert_eq!(outcome.failed_zones, failed);
    }
    assert!(outcomes.windows(2).all(|pair| {
        pair[0].surviving_nodes >= pair[1].surviving_nodes
            && (!pair[1].can_write || pair[0].can_write)
            && (!pair[1].can_read || pair[0].can_read)
    }));
}

#[test]
fn rejects_policy_mismatch_and_validates_maximum_domain_count() {
    let members = parse_membership("a east 1\nb west 1\n").unwrap();
    assert!(zone_failure_quorum_curve(QuorumPolicy::new(3, 2, 2).unwrap(), &members).is_err());
    let large = (0..16)
        .map(|index| format!("node_{index} zone_{index} 1\n"))
        .collect::<String>();
    let members = parse_membership(&large).unwrap();
    let policy = QuorumPolicy::new(16, 9, 9).unwrap();
    let curve = zone_failure_quorum_curve(policy, &members).unwrap();
    assert_eq!(curve.len(), 17);
    assert_eq!(curve[7].surviving_nodes, 9);
    assert!(curve[7].can_write);
    assert!(!curve[8].can_write);
    assert_eq!(curve[16].surviving_nodes, 0);
}
