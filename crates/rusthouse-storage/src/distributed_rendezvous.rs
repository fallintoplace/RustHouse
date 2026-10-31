//! Deterministic experimental shard-to-replica placement.
use std::collections::HashSet;

const MAX_PLACEMENT_NODES: usize = 64;

fn valid_node(node: &str) -> bool {
    !node.is_empty()
        && node.len() <= 64
        && node
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_shard(shard: &str) -> bool {
    !shard.is_empty()
        && shard.len() <= 128
        && shard.as_bytes()[0].is_ascii_alphanumeric()
        && !shard.contains("..")
        && !shard.ends_with('.')
        && shard
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

fn hash_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

fn shard_seed(shard: &str) -> u64 {
    let length = (shard.len() as u64).to_le_bytes();
    hash_bytes(hash_bytes(OFFSET, &length), shard.as_bytes())
}

fn hash_to_open_unit_interval(hash: u64) -> f64 {
    // Integer mantissas remain exact in f64. Clamping avoids both ln(0)
    // and round-to-one when the hash is at the upper endpoint.
    const MAX_MANTISSA: u64 = (1u64 << 53) - 1;
    let mantissa = (hash >> 11).clamp(1, MAX_MANTISSA);
    (mantissa as f64) / ((1u64 << 53) as f64)
}

fn score_with_seed(seed: u64, node: &str) -> u64 {
    let length = (node.len() as u64).to_le_bytes();
    hash_bytes(hash_bytes(seed, &length), node.as_bytes())
}

fn weighted_rank_order(left: &(f64, &str), right: &(f64, &str)) -> std::cmp::Ordering {
    left.0
        .total_cmp(&right.0)
        .then_with(|| left.1.cmp(right.1))
}

/// Rank up to 64 distinct nodes for one shard. This is placement, not membership consensus.
pub fn replicas_for(
    shard: &str,
    nodes: &[String],
    replicas: usize,
) -> Result<Vec<String>, &'static str> {
    if !valid_shard(shard) {
        return Err("invalid shard identifier");
    }
    if nodes.is_empty() || nodes.len() > MAX_PLACEMENT_NODES || replicas == 0 || replicas > nodes.len() {
        return Err("invalid replica placement configuration");
    }
    let mut unique = HashSet::with_capacity(nodes.len());
    for node in nodes {
        if !valid_node(node) || !unique.insert(node.as_str()) {
            return Err("invalid or duplicate replica node");
        }
    }
    let seed = shard_seed(shard);
    // A single winner does not need sorting all candidate replicas.
    if replicas == 1 {
        let mut winning_node = nodes[0].as_str();
        let mut winning_score = score_with_seed(seed, winning_node);
        for node in nodes.iter().skip(1) {
            let score = score_with_seed(seed, node);
            if score > winning_score || (score == winning_score && node.as_str() < winning_node) {
                winning_node = node;
                winning_score = score;
            }
        }
        return Ok(vec![winning_node.to_owned()]);
    }
    let mut ranked: Vec<(u64, &str)> = nodes
        .iter()
        .map(|node| (score_with_seed(seed, node), node.as_str()))
        .collect();
    if replicas < ranked.len() {
        // Partition the ranking when only a prefix is requested.
        ranked.select_nth_unstable_by(replicas, |left, right| {
            right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1))
        });
        ranked.truncate(replicas);
    }
    ranked.sort_unstable_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1)));
    Ok(ranked
        .into_iter()
        .take(replicas)
        .map(|(_, node)| node.to_owned())
        .collect())
}

/// Capacity-weighted rendezvous selection for heterogeneous replica nodes.
///
/// Uses exponential-race priorities (-ln(U)/weight), so a node with twice the
/// weight receives roughly twice the primary-placement probability. The
/// weights affect only placement; this does not establish replica consistency.
pub fn weighted_replicas_for(
    shard: &str,
    nodes: &[(&str, u16)],
    replicas: usize,
) -> Result<Vec<String>, &'static str> {
    if !valid_shard(shard)
        || nodes.is_empty()
        || nodes.len() > MAX_PLACEMENT_NODES
        || replicas == 0
        || replicas > nodes.len()
    {
        return Err("invalid weighted replica placement configuration");
    }
    let mut seen = HashSet::with_capacity(nodes.len());
    let mut ranked = Vec::with_capacity(if replicas == 1 { 0 } else { nodes.len() });
    let seed = shard_seed(shard);
    let mut winner: Option<(f64, &str)> = None;
    for &(node, weight) in nodes {
        if !valid_node(node) || !(1..=1024).contains(&weight) || !seen.insert(node) {
            return Err("invalid weighted replica node or weight");
        }
        let uniform = hash_to_open_unit_interval(score_with_seed(seed, node));
        let priority = -uniform.ln() / f64::from(weight);
        if replicas == 1 {
            if winner.is_none_or(|(best, best_node)| {
                priority
                    .total_cmp(&best)
                    .then_with(|| node.cmp(best_node))
                    .is_lt()
            }) {
                winner = Some((priority, node));
            }
        } else {
            ranked.push((priority, node));
        }
    }
    if replicas == 1 {
        return Ok(vec![winner.expect("valid nonempty nodes").1.to_owned()]);
    }
    if replicas < ranked.len() {
        // Partition first; sorting just the selected replica prefix is enough.
        ranked.select_nth_unstable_by(replicas, weighted_rank_order);
        ranked.truncate(replicas);
    }
    ranked.sort_unstable_by(weighted_rank_order);
    Ok(ranked
        .into_iter()
        .take(replicas)
        .map(|(_, node)| node.to_owned())
        .collect())
}

/// Place replicas across distinct failure zones before reusing any zone.
/// This is deterministic placement only; it does not establish consensus.
pub fn zone_aware_replicas_for(
    shard: &str,
    members: &[crate::distributed_membership::Member],
    replicas: usize,
) -> Result<Vec<String>, &'static str> {
    crate::distributed_membership::validate_membership(members)?;
    if replicas == 0 || replicas > members.len() {
        return Err("invalid zone-aware replica count");
    }
    if !valid_shard(shard) {
        return Err("invalid weighted replica placement configuration");
    }
    let seed = shard_seed(shard);
    if replicas == 1 {
        // No candidate vector or duplicate membership validation is needed
        // to select a single primary from an already validated member set.
        let mut winner: Option<(f64, usize)> = None;
        for (index, member) in members.iter().enumerate() {
            let uniform = hash_to_open_unit_interval(score_with_seed(seed, &member.node));
            let priority = -uniform.ln() / f64::from(member.weight);
            if winner.is_none_or(|(best, best_index)| {
                priority.total_cmp(&best).then_with(|| {
                    member.node.cmp(&members[best_index].node)
                }).is_lt()
            }) {
                winner = Some((priority, index));
            }
        }
        return Ok(vec![members[winner.expect("nonempty membership").1].node.clone()]);
    }
    // Rank borrowed membership indices directly. The previous path
    // revalidated nodes, allocated owned names for every candidate, and
    // built a second hash map to recover each candidate's zone.
    let mut ranked = members.iter().enumerate().map(|(index, member)| {
        let uniform = hash_to_open_unit_interval(score_with_seed(seed, &member.node));
        let priority = -uniform.ln() / f64::from(member.weight);
        (priority, index)
    }).collect::<Vec<_>>();
    ranked.sort_unstable_by(|(left, left_index), (right, right_index)| {
        left.total_cmp(right).then_with(|| {
            members[*left_index].node.cmp(&members[*right_index].node)
        })
    });
    let mut zones = HashSet::new();
    let mut selected = Vec::with_capacity(replicas);
    let mut first_pass = [false; MAX_PLACEMENT_NODES];
    for (position, &(_, index)) in ranked.iter().enumerate() {
        if zones.insert(members[index].zone.as_str()) {
            first_pass[position] = true;
            selected.push(members[index].node.clone());
            if selected.len() == replicas {
                return Ok(selected);
            }
        }
    }
    for (position, (_, index)) in ranked.into_iter().enumerate() {
        if !first_pass[position] {
            selected.push(members[index].node.clone());
            if selected.len() == replicas {
                break;
            }
        }
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membership_order_does_not_change_placement() {
        let nodes = vec!["node_a".into(), "node_b".into(), "node_c".into()];
        let reversed = nodes.iter().rev().cloned().collect::<Vec<_>>();
        let expected = replicas_for("events-2026", &nodes, 2).unwrap();
        assert_eq!(expected, replicas_for("events-2026", &reversed, 2).unwrap());
        assert_eq!(expected.len(), 2);
        assert_ne!(expected[0], expected[1]);
    }

    #[test]
    fn single_replica_path_matches_full_ranking_for_many_shards() {
        let nodes = vec!["node_c".into(), "node_a".into(), "node_b".into()];
        let weighted = [("node_c", 8u16), ("node_a", 2), ("node_b", 1)];
        for index in 0..128 {
            let shard = format!("shard_{index}");
            let best = replicas_for(&shard, &nodes, 1).unwrap();
            let ranked = replicas_for(&shard, &nodes, nodes.len()).unwrap();
            assert_eq!(best[0], ranked[0]);
            let best = weighted_replicas_for(&shard, &weighted, 1).unwrap();
            let ranked = weighted_replicas_for(&shard, &weighted, weighted.len()).unwrap();
            assert_eq!(best[0], ranked[0]);
        }
    }

    #[test]
    fn weighted_placement_is_stable_and_prefers_more_capacity() {
        let nodes = [("small", 1u16), ("large", 16u16)];
        assert_eq!(
            weighted_replicas_for("example", &nodes, 2).unwrap(),
            weighted_replicas_for(
                "example",
                &nodes.iter().rev().copied().collect::<Vec<_>>(),
                2
            )
            .unwrap()
        );
        let selected = (0..256)
            .filter(|i| {
                weighted_replicas_for(&format!("shard_{i}"), &nodes, 1).unwrap()[0] == "large"
            })
            .count();
        assert!(
            selected > 128,
            "weighted placement did not favor larger node: {selected}"
        );
        assert!(weighted_replicas_for("s", &[("n", 0)], 1).is_err());
        assert!(weighted_replicas_for("s", &[("n", 1025)], 1).is_err());
        assert!(weighted_replicas_for("s", &[("n", 1), ("n", 2)], 1).is_err());
        assert!(weighted_replicas_for("s", &nodes, 3).is_err());
    }

    #[test]
    fn rejects_unsafe_shard_names() {
        let nodes = ["node_a".to_owned()];
        for shard in [
            "../private",
            "one/two",
            ".hidden",
            "a..b",
            "a\\b",
            "é",
            "x\n",
        ] {
            assert!(replicas_for(shard, &nodes, 1).is_err(), "{shard}");
            assert!(
                weighted_replicas_for(shard, &[("node_a", 1)], 1).is_err(),
                "{shard}"
            );
        }
        assert!(replicas_for("events.2026", &nodes, 1).is_ok());
    }

    #[test]
    fn rejects_duplicate_nodes_and_bad_replica_counts() {
        assert!(replicas_for("shard", &["node".into(), "node".into()], 1).is_err());
        assert!(replicas_for("shard", &["node".into()], 2).is_err());
        assert!(replicas_for("shard", &["../bad".into()], 1).is_err());
    }
    #[test]
    fn increasing_one_node_weight_cannot_demote_its_primary_placement() {
        let baseline = [("a", 1u16), ("b", 1u16), ("c", 1u16)];
        let heavier = [("a", 64u16), ("b", 1u16), ("c", 1u16)];
        for shard in 0..512 {
            let shard = format!("events_{shard}");
            let original = weighted_replicas_for(&shard, &baseline, 1).unwrap();
            let updated = weighted_replicas_for(&shard, &heavier, 1).unwrap();
            if original[0] == "a" {
                assert_eq!(updated[0], "a", "{shard}");
            }
        }
    }

    #[test]
    fn weighted_primary_matches_full_ranking_on_many_shards() {
        let nodes = (0..64)
            .map(|index| (format!("node_{index}"), 1 + index as u16))
            .collect::<Vec<_>>();
        let weighted = nodes
            .iter()
            .map(|(node, weight)| (node.as_str(), *weight))
            .collect::<Vec<_>>();
        for index in 0..32 {
            let shard = format!("weighted_{index}");
            let first = weighted_replicas_for(&shard, &weighted, 1).unwrap();
            let all = weighted_replicas_for(&shard, &weighted, weighted.len()).unwrap();
            assert_eq!(first[0], all[0]);
        }
    }

    #[test]
    fn one_pass_primary_matches_full_rendezvous_ranking() {
        let nodes = (0..64)
            .map(|index| format!("node_{index}"))
            .collect::<Vec<_>>();
        for index in 0..64 {
            let shard = format!("shard_{index}");
            let first = replicas_for(&shard, &nodes, 1).unwrap();
            let all = replicas_for(&shard, &nodes, nodes.len()).unwrap();
            assert_eq!(first[0], all[0]);
        }
    }
}

#[cfg(test)]
mod borrowed_zone_ranking_tests {
    use super::*;
    use crate::distributed_membership::parse_membership;

    #[test]
    fn borrowed_rank_matches_full_weighted_order_across_replica_counts() {
        let manifest = (0..64)
            .map(|i| format!("n_{i} z_{} {}\n", i % 7, (i % 100) + 1))
            .collect::<String>();
        let members = parse_membership(&manifest).unwrap();
        let weights = members.iter()
            .map(|member| (member.node.as_str(), member.weight))
            .collect::<Vec<_>>();
        for shard in 0..40 {
            let shard = format!("events_{shard}");
            let rank = weighted_replicas_for(&shard, &weights, members.len()).unwrap();
            let mut zones = HashSet::new();
            let mut expected = Vec::new();
            let mut fallback = Vec::new();
            for name in rank {
                let member = members.iter().find(|member| member.node == name).unwrap();
                if zones.insert(member.zone.as_str()) {
                    expected.push(name);
                } else {
                    fallback.push(name);
                }
            }
            expected.extend(fallback);
            for replicas in [2, 3, 7, 8, 16, 64] {
                assert_eq!(
                    zone_aware_replicas_for(&shard, &members, replicas).unwrap(),
                    expected[..replicas],
                    "shard={shard}, replicas={replicas}",
                );
            }
        }
    }
}

#[cfg(test)]
mod zone_aware_tests {
    use super::*;
    use crate::distributed_membership::parse_membership;

    #[test]
    fn large_zone_membership_stays_deterministic_when_reversed() {
        let input = (0..64).map(|index| {
            format!("node_{index} zone_{} {}\n", index % 8, 1 + index % 4)
        }).collect::<String>();
        let members = parse_membership(&input).unwrap();
        let reversed = members.iter().rev().cloned().collect::<Vec<_>>();
        for shard in 0..16 {
            let name = format!("zone_test_{shard}");
            let expected = zone_aware_replicas_for(&name, &members, 8).unwrap();
            assert_eq!(expected, zone_aware_replicas_for(&name, &reversed, 8).unwrap());
            assert_eq!(expected.len(), 8);
        }
    }

    #[test]
    fn single_zone_fallback_matches_weighted_ranking_without_duplicates() {
        let membership = (0..64)
            .map(|index| format!("node_{index} same_zone {}\n", 1 + index % 4))
            .collect::<String>();
        let members = parse_membership(&membership).unwrap();
        let weighted = members
            .iter()
            .map(|member| (member.node.as_str(), member.weight))
            .collect::<Vec<_>>();
        let expected = weighted_replicas_for("fallback", &weighted, 64).unwrap();
        for replicas in [1, 2, 8, 32, 64] {
            assert_eq!(
                zone_aware_replicas_for("fallback", &members, replicas).unwrap(),
                expected[..replicas]
            );
        }
    }

    #[test]
    fn places_two_replicas_in_different_zones() {
        let members = parse_membership("a east 7\nb east 3\nc west 1\n").unwrap();
        for index in 0..32 {
            let shard = format!("shard_{index}");
            let selected = zone_aware_replicas_for(&shard, &members, 2).unwrap();
            assert_eq!(selected.len(), 2);
            assert!(selected.contains(&"c".to_owned()));
            let reversed = members.iter().rev().cloned().collect::<Vec<_>>();
            assert_eq!(selected, zone_aware_replicas_for(&shard, &reversed, 2).unwrap());
        }
        assert_eq!(zone_aware_replicas_for("s", &members, 3).unwrap().len(), 3);
        assert!(zone_aware_replicas_for("s", &members, 4).is_err());
    }
}

#[cfg(test)]
mod open_probability_endpoint_tests {
    use super::*;

    #[test]
    fn unit_interval_excludes_zero_and_one_for_all_hash_boundaries() {
        for hash in [0, 1, 2047, 2048, 4095, u64::MAX - 2048, u64::MAX] {
            let probability = hash_to_open_unit_interval(hash);
            assert!(probability > 0.0 && probability < 1.0, "{hash}: {probability}");
            assert!((-probability.ln()).is_finite());
        }
        assert_eq!(hash_to_open_unit_interval(0), 1.0 / ((1u64 << 53) as f64));
        assert_eq!(
            hash_to_open_unit_interval(u64::MAX),
            ((1u64 << 53) - 1) as f64 / ((1u64 << 53) as f64),
        );
    }
}

#[cfg(test)]
mod single_zone_aware_primary_tests {
    use super::*;
    use crate::distributed_membership::parse_membership;

    #[test]
    fn primary_matches_weighted_selection_across_zones_and_member_orders() {
        let members = parse_membership("a east 1\nb west 8\nc east 4\nd north 12\n").unwrap();
        let weights = members.iter().map(|member| (member.node.as_str(), member.weight)).collect::<Vec<_>>();
        let reversed = members.iter().rev().cloned().collect::<Vec<_>>();
        for index in 0..256 {
            let shard = format!("shard_{index}");
            let best = weighted_replicas_for(&shard, &weights, 1).unwrap();
            assert_eq!(zone_aware_replicas_for(&shard, &members, 1).unwrap(), best);
            assert_eq!(zone_aware_replicas_for(&shard, &reversed, 1).unwrap(), best);
        }
        assert!(zone_aware_replicas_for("shard", &members, 0).is_err());
    }
}

#[cfg(test)]
mod portable_shard_identifier_tests {
    use super::*;

    #[test]
    fn rejects_trailing_dot_shards_for_replicated_object_paths() {
        let nodes = vec!["node_a".to_owned(), "node_b".to_owned()];
        for shard in ["part.", "events.2026.", "a..b"] {
            assert!(replicas_for(shard, &nodes, 1).is_err(), "{shard}");
            assert!(weighted_replicas_for(shard, &[("node_a", 1)], 1).is_err(), "{shard}");
        }
        for shard in ["events.2026", "a_1", "x"] {
            assert!(replicas_for(shard, &nodes, 1).is_ok(), "{shard}");
            assert!(crate::distributed_object_key::ObjectKey::new("cluster", shard, 1, "part", "data.bin").is_ok());
        }
    }
}

/**
 * Deterministic movement plan for a placement configuration change.
 * Incoming replicas must be verified before a coordinator considers retiring
 * outgoing replicas. This only computes sets; it does not copy or delete data.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaRebalance {
    /// Desired placement order after the membership change.
    pub new_placement: Vec<String>,
    /// New destinations in desired placement order.
    pub incoming: Vec<String>,
    /// Nodes retained by both placements, in new placement order.
    pub retained: Vec<String>,
    /// Old destinations no longer selected, in previous placement order.
    pub outgoing: Vec<String>,
}

pub fn plan_zone_aware_rebalance(
    shard: &str,
    previous_members: &[crate::distributed_membership::Member],
    next_members: &[crate::distributed_membership::Member],
    replicas: usize,
) -> Result<ReplicaRebalance, &'static str> {
    let previous = zone_aware_replicas_for(shard, previous_members, replicas)?;
    let next = zone_aware_replicas_for(shard, next_members, replicas)?;
    let previous_set = previous.iter().map(String::as_str).collect::<HashSet<_>>();
    let next_set = next.iter().map(String::as_str).collect::<HashSet<_>>();
    let incoming = next.iter()
        .filter(|node| !previous_set.contains(node.as_str()))
        .cloned().collect::<Vec<_>>();
    let retained = next.iter()
        .filter(|node| previous_set.contains(node.as_str()))
        .cloned().collect::<Vec<_>>();
    let outgoing = previous.iter()
        .filter(|node| !next_set.contains(node.as_str()))
        .cloned().collect::<Vec<_>>();
    Ok(ReplicaRebalance {
        new_placement: next,
        incoming,
        retained,
        outgoing,
    })
}

#[cfg(test)]
mod replica_rebalance_tests {
    use super::*;
    use crate::distributed_membership::parse_membership;

    #[test]
    fn movement_sets_exactly_partition_old_and_new_replica_placements() {
        let old = parse_membership("a west 5\nb east 3\nc north 2\n").unwrap();
        let new = parse_membership("a west 5\nc north 2\nd south 8\ne east 1\n").unwrap();
        for replica_count in 1..=3 {
            for index in 0..128 {
                let shard = format!("shard_{index}");
                let previous = zone_aware_replicas_for(&shard, &old, replica_count).unwrap();
                let desired = zone_aware_replicas_for(&shard, &new, replica_count).unwrap();
                let plan = plan_zone_aware_rebalance(&shard, &old, &new, replica_count).unwrap();
                assert_eq!(plan.new_placement, desired);
                assert_eq!(plan.incoming.iter().chain(plan.retained.iter()).count(), replica_count);
                assert_eq!(plan.outgoing.len() + plan.retained.len(), replica_count);
                for node in &plan.incoming {
                    assert!(desired.contains(node));
                    assert!(!previous.contains(node));
                }
                for node in &plan.outgoing {
                    assert!(previous.contains(node));
                    assert!(!desired.contains(node));
                }
                for node in &plan.retained {
                    assert!(previous.contains(node));
                    assert!(desired.contains(node));
                }
                let reversed = plan_zone_aware_rebalance(&shard, &new, &old, replica_count).unwrap();
                let mut reversed_add = reversed.incoming.clone();
                let mut removed = plan.outgoing.clone();
                reversed_add.sort();
                removed.sort();
                assert_eq!(reversed_add, removed);
            }
        }
    }

    #[test]
    fn unchanged_membership_needs_no_transfers_even_if_input_is_reversed() {
        let members = parse_membership("a west 2\nb east 3\nc north 4\n").unwrap();
        let reversed = members.iter().rev().cloned().collect::<Vec<_>>();
        for count in 1..=3 {
            let plan = plan_zone_aware_rebalance("stable", &members, &reversed, count).unwrap();
            assert!(plan.incoming.is_empty());
            assert!(plan.outgoing.is_empty());
            assert_eq!(plan.retained, plan.new_placement);
        }
        assert!(plan_zone_aware_rebalance("stable", &members, &[], 1).is_err());
        assert!(plan_zone_aware_rebalance("stable", &members, &members, 0).is_err());
    }
}
