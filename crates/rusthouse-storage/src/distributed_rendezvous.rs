//! Deterministic experimental shard-to-replica placement.
use std::collections::HashSet;

fn valid_node(node: &str) -> bool {
    !node.is_empty()
        && node.len() <= 64
        && node
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn score(shard: &str, node: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let shard_length = (shard.len() as u64).to_le_bytes();
    let node_length = (node.len() as u64).to_le_bytes();
    let mut hash = OFFSET;
    for byte in shard_length
        .iter()
        .chain(shard.as_bytes())
        .chain(node_length.iter())
        .chain(node.as_bytes())
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Rank up to 64 distinct nodes for one shard. This is placement, not membership consensus.
pub fn replicas_for(
    shard: &str,
    nodes: &[String],
    replicas: usize,
) -> Result<Vec<String>, &'static str> {
    if shard.is_empty()
        || shard.len() > 128
        || !shard.as_bytes()[0].is_ascii_alphanumeric()
        || shard.contains("..")
        || !shard
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err("invalid shard identifier");
    }
    if nodes.is_empty() || nodes.len() > 64 || replicas == 0 || replicas > nodes.len() {
        return Err("invalid replica placement configuration");
    }
    let mut unique = HashSet::new();
    for node in nodes {
        if !valid_node(node) || !unique.insert(node.as_str())
        {
            return Err("invalid or duplicate replica node");
        }
    }
    let mut ranked: Vec<(u64, &str)> = nodes
        .iter()
        .map(|node| (score(shard, node), node.as_str()))
        .collect();
    ranked.sort_unstable_by(|left, right| {
        right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1))
    });
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
    if shard.is_empty() || shard.len() > 128
        || nodes.is_empty() || nodes.len() > 64
        || replicas == 0 || replicas > nodes.len()
    {
        return Err("invalid weighted replica placement configuration");
    }
    let mut seen = HashSet::new();
    let mut ranked = Vec::with_capacity(nodes.len());
    for &(node, weight) in nodes {
        if !valid_node(node) || !(1..=1024).contains(&weight) || !seen.insert(node)
        {
            return Err("invalid weighted replica node or weight");
        }
        // Keep U strictly inside (0, 1). Using the top 53 bits avoids
        // rounding a 64-bit hash to 1.0 in f64.
        let uniform = ((score(shard, node) >> 11) as f64 + 0.5)
            / ((1u64 << 53) as f64);
        let priority = -uniform.ln() / f64::from(weight);
        ranked.push((priority, node));
    }
    ranked.sort_unstable_by(|left, right| {
        left.0.total_cmp(&right.0).then_with(|| left.1.cmp(right.1))
    });
    Ok(ranked.into_iter().take(replicas)
        .map(|(_, node)| node.to_owned()).collect())
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
    fn weighted_placement_is_stable_and_prefers_more_capacity() {
        let nodes = [("small", 1u16), ("large", 16u16)];
        assert_eq!(
            weighted_replicas_for("example", &nodes, 2).unwrap(),
            weighted_replicas_for("example", &nodes.iter().rev().copied().collect::<Vec<_>>(), 2).unwrap()
        );
        let selected = (0..256).filter(|i| {
            weighted_replicas_for(&format!("shard_{i}"), &nodes, 1).unwrap()[0] == "large"
        }).count();
        assert!(selected > 128, "weighted placement did not favor larger node: {selected}");
        assert!(weighted_replicas_for("s", &[("n", 0)], 1).is_err());
        assert!(weighted_replicas_for("s", &[("n", 1025)], 1).is_err());
        assert!(weighted_replicas_for("s", &[("n", 1), ("n", 2)], 1).is_err());
        assert!(weighted_replicas_for("s", &nodes, 3).is_err());
    }

    #[test]
    fn rejects_unsafe_shard_names() {
        let nodes = ["node_a".to_owned()];
        for shard in ["../private", "one/two", ".hidden", "a..b", "a\\b", "é", "x\n"] {
            assert!(replicas_for(shard, &nodes, 1).is_err(), "{shard}");
        }
        assert!(replicas_for("events.2026", &nodes, 1).is_ok());
    }

    #[test]
    fn rejects_duplicate_nodes_and_bad_replica_counts() {
        assert!(replicas_for("shard", &["node".into(), "node".into()], 1).is_err());
        assert!(replicas_for("shard", &["node".into()], 2).is_err());
        assert!(replicas_for("shard", &["../bad".into()], 1).is_err());
    }
}