//! Rank candidate storage replicas for reads without claiming consistency.
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaStatus {
    pub node: String,
    pub epoch: u64,
    pub generation: u64,
    pub healthy: bool,
}

fn valid_node(node: &str) -> bool {
    !node.is_empty()
        && node.len() <= 64
        && node
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn newest_first(left: &ReplicaStatus, right: &ReplicaStatus) -> std::cmp::Ordering {
    right
        .epoch
        .cmp(&left.epoch)
        .then_with(|| right.generation.cmp(&left.generation))
        .then_with(|| left.node.cmp(&right.node))
}

/// Return healthy node IDs, newest advertised version first.
/// A caller must still validate returned data and enforce its consistency model.
pub fn rank_read_replicas(replicas: &[ReplicaStatus]) -> Result<Vec<String>, &'static str> {
    if replicas.is_empty() || replicas.len() > 64 {
        return Err("invalid replica status set");
    }
    let mut seen = HashSet::new();
    for replica in replicas {
        if !valid_node(&replica.node) || !seen.insert(replica.node.as_str())
        {
            return Err("invalid or duplicate replica node");
        }
    }
    let mut healthy: Vec<&ReplicaStatus> = replicas
        .iter()
        .filter(|replica| replica.healthy)
        .collect();
    if healthy.is_empty() {
        return Err("no healthy storage replica");
    }
    healthy.sort_unstable_by(newest_first);
    Ok(healthy.into_iter().map(|replica| replica.node.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_newest_healthy_replica() {
        let replicas = vec![
            ReplicaStatus { node: "a".into(), epoch: 3, generation: 10, healthy: false },
            ReplicaStatus { node: "b".into(), epoch: 3, generation: 9, healthy: true },
            ReplicaStatus { node: "c".into(), epoch: 4, generation: 1, healthy: true },
        ];
        assert_eq!(rank_read_replicas(&replicas).unwrap(), vec!["c", "b"]);
    }

    #[test]
    fn rejects_missing_or_duplicate_nodes() {
        let one = ReplicaStatus { node: "a".into(), epoch: 1, generation: 1, healthy: false };
        assert!(rank_read_replicas(&[one.clone()]).is_err());
        assert!(rank_read_replicas(&[one.clone(), one]).is_err());
    }
    #[test]
    fn ties_sort_by_node_and_unhealthy_nodes_cannot_win() {
        let replicas = vec![
            ReplicaStatus { node: "b".into(), epoch: 2, generation: 3, healthy: true },
            ReplicaStatus { node: "a".into(), epoch: 2, generation: 3, healthy: true },
            ReplicaStatus { node: "old".into(), epoch: 1, generation: 99, healthy: true },
            ReplicaStatus { node: "new".into(), epoch: 3, generation: 9, healthy: false },
        ];
        assert_eq!(rank_read_replicas(&replicas).unwrap(), vec!["a", "b", "old"]);
        assert!(rank_read_replicas(&[
            ReplicaStatus { node: "../bad".into(), epoch: 1, generation: 1, healthy: false },
            replicas[0].clone(),
        ]).is_err());
    }

}