use super::*;

#[test]
fn shared_replica_reaches_both_quorums_with_one_new_vote() {
    let policy = QuorumPolicy::new(3, 2, 2).unwrap();
    let old = ["a", "b", "c"];
    let new = ["b", "c", "d"];
    assert_eq!(
        plan_joint_write_acknowledgements(policy, &old, policy, &new, &["a", "d"]).unwrap(),
        vec!["b"],
    );
    assert_eq!(
        plan_joint_write_acknowledgements(policy, &old, policy, &new, &["b"]).unwrap(),
        vec!["c"],
    );
    assert!(
        plan_joint_write_acknowledgements(policy, &old, policy, &new, &["a", "b", "c", "d"],)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn already_satisfied_membership_requires_only_missing_votes_from_other_side() {
    let policy = QuorumPolicy::new(3, 2, 2).unwrap();
    let previous = ["a", "b", "c"];
    let next = ["b", "c", "d"];
    assert_eq!(
        plan_joint_write_acknowledgements(policy, &previous, policy, &next, &["a", "b"])
            .unwrap(),
        ["c"]
    );
    assert_eq!(
        plan_joint_write_acknowledgements(policy, &previous, policy, &next, &["b", "d"])
            .unwrap(),
        ["a"]
    );
    assert_eq!(
        plan_joint_write_acknowledgements(policy, &previous, policy, &next, &["a", "d"])
            .unwrap(),
        ["b"]
    );
}

#[test]
fn exhaustive_small_configurations_match_brute_force_minimum_and_ties() {
    let names = ["a", "b", "c", "d"];
    let policy = QuorumPolicy::new(3, 2, 2).unwrap();
    for old in [["a", "b", "c"], ["a", "b", "d"], ["b", "c", "d"]] {
        for new in [["a", "b", "c"], ["a", "c", "d"], ["b", "c", "d"]] {
            for acknowledged_mask in 0u32..16 {
                let acknowledged = names
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &name)| {
                        (acknowledged_mask & (1 << i) != 0).then_some(name)
                    })
                    .collect::<Vec<_>>();
                if !acknowledged
                    .iter()
                    .all(|name| old.contains(name) || new.contains(name))
                {
                    continue;
                }
                let selected = plan_joint_write_acknowledgements(
                    policy,
                    &old,
                    policy,
                    &new,
                    &acknowledged,
                )
                .unwrap();
                let mut expected: Option<Vec<&str>> = None;
                for subset in 0u32..16 {
                    if subset & acknowledged_mask != 0 {
                        continue;
                    }
                    let extra = names
                        .iter()
                        .enumerate()
                        .filter_map(|(i, &name)| (subset & (1 << i) != 0).then_some(name))
                        .collect::<Vec<_>>();
                    if !extra
                        .iter()
                        .all(|name| old.contains(name) || new.contains(name))
                    {
                        continue;
                    }
                    let mut votes = acknowledged.clone();
                    votes.extend(&extra);
                    if !observe_joint_quorum(policy, &old, policy, &new, &votes)
                        .unwrap()
                        .can_write
                    {
                        continue;
                    }
                    if expected.as_ref().is_none_or(|best| {
                        extra.len() < best.len() || (extra.len() == best.len() && extra < *best)
                    }) {
                        expected = Some(extra);
                    }
                }
                assert_eq!(
                    selected,
                    expected
                        .unwrap()
                        .iter()
                        .map(|name| (*name).to_owned())
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}

#[test]
fn invalid_configurations_and_unknown_acknowledgements_fail_closed() {
    let policy = QuorumPolicy::new(3, 2, 2).unwrap();
    assert!(
        plan_joint_write_acknowledgements(
            policy,
            &["a", "a", "c"],
            policy,
            &["b", "c", "d"],
            &[],
        )
        .is_err()
    );
    assert!(
        plan_joint_write_acknowledgements(
            policy,
            &["a", "b", "c"],
            policy,
            &["b", "c", "d"],
            &["x"],
        )
        .is_err()
    );
}
