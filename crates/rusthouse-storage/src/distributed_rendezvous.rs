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

fn score_with_seed(seed: u64, node: &str) -> u64 {
    let length = (node.len() as u64).to_le_bytes();
    hash_bytes(hash_bytes(seed, &length), node.as_bytes())
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
        let mut winner = nodes[0].as_str();
        let mut winner_score = score_with_seed(seed, winner);
        for node in nodes.iter().skip(1) {
            let score = score_with_seed(seed, node);
            if score > winner_score || (score == winner_score && node.as_str() < winner) {
                winner = node;
                winner_score = score;
            }
        }
        return Ok(vec![winner.to_owned()]);
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
        // Keep U strictly inside (0, 1). Using the top 53 bits avoids
        // rounding a 64-bit hash to 1.0 in f64.
        let uniform = ((score_with_seed(seed, node) >> 11) as f64 + 0.5) / ((1u64 << 53) as f64);
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
        ranked.select_nth_unstable_by(replicas, |left, right| {
            left.0.total_cmp(&right.0).then_with(|| left.1.cmp(right.1))
        });
        ranked.truncate(replicas);
    }
    ranked.sort_unstable_by(|left, right| {
        left.0.total_cmp(&right.0).then_with(|| left.1.cmp(right.1))
    });
    Ok(ranked
        .into_iter()
        .take(replicas)
        .map(|(_, node)| node.to_owned())
        .collect())
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

/// Place replicas across distinct failure zones before reusing any zone.
/// This is deterministic placement only; it does not establish consensus.
pub fn zone_aware_replicas_for(
    shard: &str,
    members: &[crate::distributed_membership::Member],
    replicas: usize,
) -> Result<Vec<String>, &'static str> {
    crate::distributed_membership::format_membership(members)?;
    if replicas == 0 || replicas > members.len() {
        return Err("invalid zone-aware replica count");
    }
    let nodes = members
        .iter()
        .map(|member| (member.node.as_str(), member.weight))
        .collect::<Vec<_>>();
    let ranked = weighted_replicas_for(shard, &nodes, members.len())?;
    let mut zones = HashSet::new();
    let mut selected = Vec::with_capacity(replicas);
    for node in &ranked {
        let zone = members
            .iter()
            .find(|member| member.node == *node)
            .expect("ranked node belongs to validated membership")
            .zone
            .as_str();
        if zones.insert(zone) {
            selected.push(node.clone());
            if selected.len() == replicas {
                return Ok(selected);
            }
        }
    }
    for node in ranked {
        if !selected.contains(&node) {
            selected.push(node);
            if selected.len() == replicas {
                break;
            }
        }
    }
    Ok(selected)
}

#[cfg(test)]
mod zone_aware_tests {
    use super::*;
    use crate::distributed_membership::parse_membership;

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
