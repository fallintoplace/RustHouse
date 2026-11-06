//! Quorum policy validation for future replicated storage writes.
//! This module does not implement consensus or distributed transactions.
mod policy;
pub use policy::{
    QuorumObservation,
    QuorumPolicy,
    ZoneFailureQuorum,
    QuorumDeficit,
    worst_case_zone_quorum,
};

fn valid_member(member: &str) -> bool {
    !member.is_empty()
        && member.len() <= 64
        && member
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}


/**
 * Report whether acknowledgements satisfy both quorum configurations during
 * a membership change. This is a pure planner, not an election or consensus
 * implementation. One shared node counts once in each configuration.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JointQuorumObservation {
    pub previous: QuorumObservation,
    pub next: QuorumObservation,
    pub can_read: bool,
    pub can_write: bool,
}

pub fn observe_joint_quorum(
    previous: QuorumPolicy,
    previous_members: &[&str],
    next: QuorumPolicy,
    next_members: &[&str],
    acknowledgements: &[&str],
) -> Result<JointQuorumObservation, &'static str> {
    // Large batches index both memberships together below. Validate their
    // policies here, but avoid building two temporary membership hash maps.
    previous.validate()?;
    next.validate()?;
    if previous_members.len() != previous.replicas || next_members.len() != next.replicas {
        return Err("quorum membership or acknowledgement count mismatch");
    }
    if acknowledgements.len() > 256 {
        return Err("quorum acknowledgement count exceeds bound");
    }
    if acknowledgements.len() <= 32 {
        // Small acknowledgement batches use borrowed linear probes. Validate
        // both memberships even when no acknowledgements were received.
        previous.observe(previous_members, &[])?;
        next.observe(next_members, &[])?;
    }
    let mut previous_mask = 0u64;
    let mut next_mask = 0u64;
    if acknowledgements.len() > 32 {
        // Large retry batches often repeat the same acknowledgement. Resolve
        // a replica ID to its old/new bit positions only once, while sharing
        // nodes count as one vote in each membership configuration.
        // A policy has at most 64 members on each side. Build the
        // combined old/new ID index on the stack, then binary-search up to
        // 256 acknowledgements without allocating a hash table.
        let mut entries = [("", 0u64, 0u64); 128];
        let mut len = 0usize;
        for (index, &member) in previous_members.iter().enumerate() {
            if !valid_member(member) {
                return Err("invalid or duplicate quorum member");
            }
            entries[len] = (member, 1u64 << index, 0);
            len += 1;
        }
        for (index, &member) in next_members.iter().enumerate() {
            if !valid_member(member) {
                return Err("invalid or duplicate quorum member");
            }
            entries[len] = (member, 0, 1u64 << index);
            len += 1;
        }
        entries[..len].sort_unstable_by(|left, right| left.0.cmp(right.0));
        let mut index = [("", 0u64, 0u64); 128];
        let mut unique = 0usize;
        for (member, old, new) in entries[..len].iter().copied() {
            if unique > 0 && index[unique - 1].0 == member {
                let previous = &mut index[unique - 1];
                // Duplicates across configurations are legal; duplicates
                // within the same configuration must still be rejected.
                if (old != 0 && previous.1 != 0) || (new != 0 && previous.2 != 0) {
                    return Err("invalid or duplicate quorum member");
                }
                previous.1 |= old;
                previous.2 |= new;
            } else {
                index[unique] = (member, old, new);
                unique += 1;
            }
        }
        let sorted = &index[..unique];
        for &member in acknowledgements {
            let position = sorted
                .binary_search_by(|entry| entry.0.cmp(member))
                .map_err(|_| "acknowledgement from unknown replica")?;
            previous_mask |= sorted[position].1;
            next_mask |= sorted[position].2;
        }
    } else {
        // Small batches avoid a temporary map entirely.
        for &member in acknowledgements {
            let old = previous_members.iter().position(|&id| id == member);
            let new = next_members.iter().position(|&id| id == member);
            if old.is_none() && new.is_none() {
                return Err("acknowledgement from unknown replica");
            }
            if let Some(index) = old {
                previous_mask |= 1u64 << index;
            }
            if let Some(index) = new {
                next_mask |= 1u64 << index;
            }
        }
    }
    let previous_count = previous_mask.count_ones() as usize;
    let next_count = next_mask.count_ones() as usize;
    let previous = QuorumObservation {
        distinct_acks: previous_count,
        can_read: previous_count >= previous.reads,
        can_write: previous_count >= previous.writes,
    };
    let next = QuorumObservation {
        distinct_acks: next_count,
        can_read: next_count >= next.reads,
        can_write: next_count >= next.writes,
    };
    Ok(JointQuorumObservation {
        previous,
        next,
        can_read: previous.can_read && next.can_read,
        can_write: previous.can_write && next.can_write,
    })
}

#[cfg(test)]
mod joint_reconfiguration_tests {
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
}

#[cfg(test)]
mod joint_quorum_large_ack_index_tests {
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
}

#[cfg(test)]
mod indexed_joint_membership_validation_tests {
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
}

#[cfg(test)]
mod sparse_large_quorum_ack_tests {
    use super::*;

    #[test]
    fn sparse_acks_match_full_member_count_and_preserve_errors() {
        let nodes = (0..16)
            .map(|index| format!("replica_{index}"))
            .collect::<Vec<_>>();
        let names = nodes.iter().map(String::as_str).collect::<Vec<_>>();
        let policy = QuorumPolicy::new(16, 8, 9).unwrap();
        for count in [0, 1, 2, 7, 8, 9, 16] {
            let observed = policy.observe(&names, &names[..count]).unwrap();
            assert_eq!(observed.distinct_acks, count);
            assert_eq!(observed.can_read, count >= 8);
            assert_eq!(observed.can_write, count >= 9);
        }
        assert_eq!(
            policy
                .observe(&names, &["replica_0"; 8])
                .unwrap()
                .distinct_acks,
            1
        );
        assert!(policy.observe(&names, &["missing"]).is_err());
        let mut duplicate = names.clone();
        duplicate[15] = duplicate[0];
        assert!(policy.observe(&duplicate, &[]).is_err());
        assert!(policy.observe(&duplicate, &names[..9]).is_err());
    }
}

#[cfg(test)]
mod stack_quorum_index_tests {
    use super::*;

    #[test]
    fn many_acknowledgements_match_reference_masks_in_every_large_group() {
        for replicas in 9..=16 {
            let names = (0..replicas)
                .map(|n| format!("member_{n}"))
                .collect::<Vec<_>>();
            let members = names.iter().map(String::as_str).collect::<Vec<_>>();
            let policy = QuorumPolicy::new(replicas, 1, replicas).unwrap();
            for seed in 0..32usize {
                let acks = (0..256)
                    .map(|n| members[(n * 37 + seed) % replicas])
                    .collect::<Vec<_>>();
                let reference = (0..replicas)
                    .filter(|&i| acks.contains(&members[i]))
                    .count();
                let observed = policy.observe(&members, &acks).unwrap();
                assert_eq!(observed.distinct_acks, reference);
                assert_eq!(observed.can_write, reference >= replicas);
            }
            let mut duplicates = members.clone();
            duplicates[replicas - 1] = duplicates[0];
            assert!(policy.observe(&duplicates, &members).is_err());
        }
    }
}

#[cfg(test)]
mod stack_joint_quorum_index_tests {
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
}

#[cfg(test)]
mod failure_zone_quorum_reference_tests {
    use super::*;
    use crate::distributed_membership::Member;

    fn membership(zones: &[usize]) -> Vec<Member> {
        zones
            .iter()
            .enumerate()
            .flat_map(|(zone, &count)| {
                (0..count).map(move |index| Member {
                    node: format!("node_{zone}_{index}"),
                    zone: format!("zone_{zone}"),
                    weight: 1,
                })
            })
            .collect()
    }

    #[test]
    fn worst_case_matches_exhaustive_zone_failure_subsets() {
        for counts in [
            &[1usize, 1, 1][..],
            &[1, 2, 1, 1],
            &[4, 1, 2],
            &[2, 3, 1, 2, 1],
            &[3, 3, 3, 2, 3, 2],
        ] {
            let members = membership(counts);
            let n = members.len();
            let writes = n / 2 + 1;
            let reads = n - writes + 1;
            let policy = QuorumPolicy::new(n, reads, writes).unwrap();
            for failed in 0..=counts.len() {
                let mut most_lost = 0usize;
                for subset in 0..(1usize << counts.len()) {
                    if subset.count_ones() as usize != failed {
                        continue;
                    }
                    let lost = counts
                        .iter()
                        .enumerate()
                        .filter(|(zone, _)| subset & (1 << zone) != 0)
                        .map(|(_, &size)| size)
                        .sum::<usize>();
                    most_lost = most_lost.max(lost);
                }
                let expected = n - most_lost;
                let result = worst_case_zone_quorum(policy, &members, failed).unwrap();
                assert_eq!(
                    result.surviving_nodes, expected,
                    "zones={counts:?} failed={failed}"
                );
                assert_eq!(result.can_read, expected >= reads);
                assert_eq!(result.can_write, expected >= writes);
            }
        }
    }

    #[test]
    fn rejects_bad_membership_and_out_of_range_zone_failures() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let members = membership(&[2, 1]);
        assert!(worst_case_zone_quorum(policy, &members, 3).is_err());
        assert_eq!(
            worst_case_zone_quorum(policy, &members, 0)
                .unwrap()
                .surviving_nodes,
            3
        );
        let wrong = membership(&[2, 2]);
        assert!(worst_case_zone_quorum(policy, &wrong, 1).is_err());
        let mut repeated = members.clone();
        repeated[2].node = repeated[0].node.clone();
        assert!(worst_case_zone_quorum(policy, &repeated, 0).is_err());
    }
}

/// Pick the smallest set of additional replica acknowledgements that would
/// satisfy *both* write quorums during joint membership reconfiguration.
///
/// This is a planning helper, not a lease, acknowledgement, or consensus
/// operation. Ties choose the lexicographically earliest set of replica IDs.
/// A shared replica can satisfy one vote in each membership with one request.
pub fn plan_joint_write_acknowledgements(
    previous: QuorumPolicy,
    previous_members: &[&str],
    next: QuorumPolicy,
    next_members: &[&str],
    acknowledgements: &[&str],
) -> Result<Vec<String>, &'static str> {
    let observed = observe_joint_quorum(
        previous,
        previous_members,
        next,
        next_members,
        acknowledgements,
    )?;
    let previous_missing = previous
        .writes
        .saturating_sub(observed.previous.distinct_acks);
    let next_missing = next.writes.saturating_sub(observed.next.distinct_acks);
    if previous_missing == 0 && next_missing == 0 {
        return Ok(Vec::new());
    }

    let mut candidates = previous_members
        .iter()
        .chain(next_members)
        .copied()
        .filter(|member| !acknowledgements.contains(member))
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    candidates.dedup();
    // A membership that already has write quorum imposes no further
    // constraint. The lexicographically earliest remaining members from the
    // other membership are the exact minimum, without a two-dimensional DP.
    if previous_missing == 0 || next_missing == 0 {
        let (missing, members) = if previous_missing == 0 {
            (next_missing, next_members)
        } else {
            (previous_missing, previous_members)
        };
        return Ok(candidates
            .into_iter()
            .filter(|candidate| members.contains(candidate))
            .take(missing)
            .map(str::to_owned)
            .collect());
    }
    // One common unacknowledged member can supply the final vote in both
    // memberships. Prefer the smallest common name for the exact tie break.
    if previous_missing == 1 && next_missing == 1 {
        if let Some(&shared) = candidates
            .iter()
            .find(|&&name| previous_members.contains(&name) && next_members.contains(&name))
        {
            return Ok(vec![shared.to_owned()]);
        }
    }
    // Policies permit up to 64 members on either side. A 128-bit mask
    // represents every candidate without heap-backed DP paths.
    debug_assert!(candidates.len() <= 128);
    let width = next_missing + 1;
    let mut best = vec![None::<u128>; (previous_missing + 1) * width];
    best[0] = Some(0);
    for (index, candidate) in candidates.iter().enumerate() {
        let in_previous = usize::from(previous_members.contains(candidate));
        let in_next = usize::from(next_members.contains(candidate));
        for old in (0..=previous_missing).rev() {
            for new in (0..=next_missing).rev() {
                let Some(mask) = best[old * width + new] else {
                    continue;
                };
                let updated_old = (old + in_previous).min(previous_missing);
                let updated_new = (new + in_next).min(next_missing);
                if updated_old == old && updated_new == new {
                    continue;
                }
                let updated_mask = mask | (1u128 << index);
                let slot = &mut best[updated_old * width + updated_new];
                let replace = match *slot {
                    None => true,
                    Some(existing) if updated_mask.count_ones() < existing.count_ones() => true,
                    Some(existing) if updated_mask.count_ones() == existing.count_ones() => {
                        // Candidates are sorted. The set containing the first
                        // differing candidate is the lexicographically first.
                        let difference = updated_mask ^ existing;
                        difference != 0
                            && updated_mask & (1u128 << difference.trailing_zeros()) != 0
                    }
                    _ => false,
                };
                if replace {
                    *slot = Some(updated_mask);
                }
            }
        }
    }
    let selected = best[previous_missing * width + next_missing]
        .ok_or("joint write quorum cannot be reached")?;
    Ok(candidates
        .iter()
        .enumerate()
        .filter(|&(index, _)| selected & (1u128 << index) != 0)
        .map(|(_, &candidate)| candidate.to_owned())
        .collect())
}

#[cfg(test)]
mod optimal_joint_write_ack_tests {
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
}

#[cfg(test)]
mod wide_quorum_support_tests {
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
}

/// Compute every worst-case failure-zone quorum outcome in a single pass.
///
/// The k-th entry describes losing the k largest distinct failure zones.
/// This is an availability envelope, not proof that any replica completed
/// consensus or that data written by a quorum was durably persisted.
pub fn zone_failure_quorum_curve(
    policy: QuorumPolicy,
    members: &[crate::distributed_membership::Member],
) -> Result<Vec<ZoneFailureQuorum>, &'static str> {
    policy.validate()?;
    crate::distributed_membership::validate_membership(members)?;
    if members.len() != policy.replicas {
        return Err("quorum policy does not match failure-zone membership");
    }
    // Validated quorum policies have at most 16 nodes, so accumulating
    // borrowed zone names is cheaper than building a hash table.
    let mut zones = Vec::<(&str, usize)>::new();
    for member in members {
        match zones
            .iter_mut()
            .find(|(zone, _)| *zone == member.zone.as_str())
        {
            Some((_, count)) => *count += 1,
            None => zones.push((&member.zone, 1)),
        }
    }
    zones.sort_unstable_by_key(|&(_, nodes)| std::cmp::Reverse(nodes));
    let mut surviving = members.len();
    let mut outcomes = Vec::with_capacity(zones.len() + 1);
    outcomes.push(ZoneFailureQuorum {
        failed_zones: 0,
        surviving_nodes: surviving,
        can_read: policy.can_read(surviving),
        can_write: policy.can_write(surviving),
    });
    for (index, (_, lost)) in zones.iter().enumerate() {
        surviving -= lost;
        outcomes.push(ZoneFailureQuorum {
            failed_zones: index + 1,
            surviving_nodes: surviving,
            can_read: policy.can_read(surviving),
            can_write: policy.can_write(surviving),
        });
    }
    Ok(outcomes)
}

/// Restore the fewest down failure domains needed to regain each
/// quorum threshold. Zone choices are sorted by survivor count, then name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneRecoveryPlan {
    pub surviving_replicas: usize,
    pub restore_for_reads: Vec<String>,
    pub restore_for_writes: Vec<String>,
}

/// Plan availability recovery from a concrete set of failed zones.
///
/// Every listed zone must exist and be unique. This is only a quorum-capacity
/// calculation, not proof that recovering a node restores a durable snapshot.
pub fn plan_zone_quorum_recovery(
    policy: QuorumPolicy,
    members: &[crate::distributed_membership::Member],
    down_zones: &[&str],
) -> Result<ZoneRecoveryPlan, &'static str> {
    policy.validate()?;
    crate::distributed_membership::validate_membership(members)?;
    if members.len() != policy.replicas || down_zones.len() > members.len() {
        return Err("invalid zone recovery membership");
    }
    let mut zones = Vec::<(&str, usize)>::new();
    for member in members {
        if let Some((_, count)) = zones
            .iter_mut()
            .find(|(zone, _)| *zone == member.zone.as_str())
        {
            *count += 1;
        } else {
            zones.push((&member.zone, 1));
        }
    }
    let mut failed = Vec::<(&str, usize)>::new();
    for &zone in down_zones {
        let count = zones
            .iter()
            .find(|(known, _)| *known == zone)
            .ok_or("unknown failure zone")?
            .1;
        if failed.iter().any(|(known, _)| *known == zone) {
            return Err("duplicate failure zone");
        }
        failed.push((zone, count));
    }
    let lost = failed.iter().map(|(_, count)| *count).sum::<usize>();
    let surviving_replicas = members.len() - lost;
    failed.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    fn restore(failed: &[(&str, usize)], missing: usize) -> Vec<String> {
        let mut restored = Vec::new();
        let mut nodes = 0usize;
        for &(zone, count) in failed {
            if nodes >= missing {
                break;
            }
            restored.push(zone.to_owned());
            nodes += count;
        }
        restored
    }
    Ok(ZoneRecoveryPlan {
        surviving_replicas,
        restore_for_reads: restore(&failed, policy.reads.saturating_sub(surviving_replicas)),
        restore_for_writes: restore(&failed, policy.writes.saturating_sub(surviving_replicas)),
    })
}

#[cfg(test)]
mod actual_zone_quorum_recovery_tests {
    use super::*;
    use crate::distributed_membership::parse_membership;

    #[test]
    fn recovery_chooses_minimum_number_of_lost_zones() {
        let members = parse_membership(
            "a east 1\nb east 1\nc east 1\nd west 1\ne west 1\nf north 1\ng south 1\n",
        )
        .unwrap();
        let policy = QuorumPolicy::new(7, 4, 4).unwrap();
        let zones = ["east", "west", "north", "south"];
        for mask in 0usize..16 {
            let down = zones
                .iter()
                .enumerate()
                .filter_map(|(i, zone)| ((mask >> i) & 1 != 0).then_some(*zone))
                .collect::<Vec<_>>();
            let plan = plan_zone_quorum_recovery(policy, &members, &down).unwrap();
            let surviving = members
                .iter()
                .filter(|m| !down.contains(&m.zone.as_str()))
                .count();
            assert_eq!(plan.surviving_replicas, surviving);
            for (selected, threshold) in [
                (&plan.restore_for_reads, policy.reads),
                (&plan.restore_for_writes, policy.writes),
            ] {
                let regained = selected
                    .iter()
                    .map(|zone| members.iter().filter(|m| m.zone == *zone).count())
                    .sum::<usize>();
                assert!(surviving + regained >= threshold);
                let mut min_count = usize::MAX;
                for subset in 0usize..(1usize << down.len()) {
                    let added = down
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| subset & (1usize << *i) != 0)
                        .map(|(_, zone)| members.iter().filter(|m| m.zone == *zone).count())
                        .sum::<usize>();
                    if surviving + added >= threshold {
                        min_count = min_count.min(subset.count_ones() as usize);
                    }
                }
                assert_eq!(selected.len(), min_count);
            }
        }
    }

    #[test]
    fn unknown_duplicate_and_mismatched_outages_are_rejected() {
        let members = parse_membership("a east 1\nb west 1\nc south 1\n").unwrap();
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let result = plan_zone_quorum_recovery(policy, &members, &["east", "west"]);
        assert_eq!(result.unwrap().restore_for_reads, vec!["east".to_owned()]);
        assert!(plan_zone_quorum_recovery(policy, &members, &["unknown"]).is_err());
        assert!(plan_zone_quorum_recovery(policy, &members, &["east", "east"]).is_err());
        assert!(
            plan_zone_quorum_recovery(QuorumPolicy::new(4, 3, 3).unwrap(), &members, &[],).is_err()
        );
    }
}

#[cfg(test)]
mod batched_zone_failure_quorum_tests {
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
}
