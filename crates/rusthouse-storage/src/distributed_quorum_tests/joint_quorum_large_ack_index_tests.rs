use super::*;

#[test]
fn indexed_acknowledgements_match_small_batch_observations() {
    let policy = QuorumPolicy::new(16, 9, 9).unwrap();
    let old_nodes = (0..16)
        .map(|index| format!("node_{index:02}"))
        .collect::<Vec<_>>();
    let new_nodes = (8..24)
        .map(|index| format!("node_{index:02}"))
        .collect::<Vec<_>>();
    let old = old_nodes.iter().map(String::as_str).collect::<Vec<_>>();
    let new = new_nodes.iter().map(String::as_str).collect::<Vec<_>>();
    for count in 0..=24usize {
        let acknowledgements = (0..count)
            .map(|index| format!("node_{index:02}"))
            .collect::<Vec<_>>();
        let unique = acknowledgements
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let mut repeated = Vec::new();
        for _ in 0..10 {
            repeated.extend(unique.iter().copied());
        }
        if repeated.len() <= 32 {
            continue;
        }
        let expected = observe_joint_quorum(policy, &old, policy, &new, &unique).unwrap();
        assert_eq!(
            observe_joint_quorum(policy, &old, policy, &new, &repeated).unwrap(),
            expected,
            "distinct acknowledgements: {count}",
        );
    }
    let too_many = vec!["node_08"; 257];
    assert!(observe_joint_quorum(policy, &old, policy, &new, &too_many).is_err());
    let mut bad = vec!["node_08"; 40];
    bad.push("stranger");
    assert!(observe_joint_quorum(policy, &old, policy, &new, &bad).is_err());
}
