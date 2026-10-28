//! Deterministic experimental shard-to-replica placement.
use std::collections::HashSet;

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
    if shard.is_empty() || shard.len() > 128 {
        return Err("invalid shard identifier");
    }
    if nodes.is_empty() || nodes.len() > 64 || replicas == 0 || replicas > nodes.len() {
        return Err("invalid replica placement configuration");
    }
    let mut unique = HashSet::new();
    for node in nodes {
        if node.is_empty()
            || node.len() > 64
            || !node
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            || !unique.insert(node.as_str())
        {
            return Err("invalid or duplicate replica node");
        }
    }
    let mut ranked: Vec<(u64, &str)> = nodes
        .iter()
        .map(|node| (score(shard, node), node.as_str()))
        .collect();
    ranked.sort_unstable_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1)));
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
    fn rejects_duplicate_nodes_and_bad_replica_counts() {
        assert!(replicas_for("shard", &["node".into(), "node".into()], 1).is_err());
        assert!(replicas_for("shard", &["node".into()], 2).is_err());
        assert!(replicas_for("shard", &["../bad".into()], 1).is_err());
    }
}
