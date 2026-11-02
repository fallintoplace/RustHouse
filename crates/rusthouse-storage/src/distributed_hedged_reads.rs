//! Deterministic hedged-read plan for an externally fixed snapshot.
//! Scheduling does not issue network requests or establish replica freshness.
use crate::distributed_lineage::SnapshotVersion;
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HedgedReplica {
    pub node: String,
    pub zone: String,
    pub healthy: bool,
    pub p95_latency_ms: u64,
    pub snapshot: SnapshotVersion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HedgedAttempt {
    pub node: String,
    pub at_ms: u64,
}

fn valid_replica_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub fn plan_hedged_reads(
    replicas: &[HedgedReplica],
    snapshot: SnapshotVersion,
    deadline_ms: u64,
    hedge_after_ms: u64,
    max_attempts: usize,
) -> Result<Vec<HedgedAttempt>, &'static str> {
    if replicas.is_empty() || replicas.len() > 64
        || snapshot.epoch == 0 || snapshot.generation == 0
        || deadline_ms == 0 || hedge_after_ms == 0
        || max_attempts == 0 || max_attempts > 8
    {
        return Err("invalid hedged read limits or snapshot");
    }
    let mut names = HashSet::new();
    for replica in replicas {
        if !valid_replica_label(&replica.node)
            || !valid_replica_label(&replica.zone)
            || replica.p95_latency_ms == 0 || !names.insert(&replica.node)
            || replica.snapshot.epoch == 0 || replica.snapshot.generation == 0
        {
            return Err("invalid or duplicate hedged replica");
        }
        if replica.healthy
            && replica.snapshot.epoch == snapshot.epoch
            && replica.snapshot.generation == snapshot.generation
            && replica.snapshot.fingerprint != snapshot.fingerprint
        {
            return Err("conflicting replica snapshot fingerprint");
        }
    }
    let mut eligible = replicas.iter()
        .filter(|replica| replica.healthy && replica.snapshot == snapshot)
        .collect::<Vec<_>>();
    eligible.sort_unstable_by(|a, b| {
        a.p95_latency_ms.cmp(&b.p95_latency_ms).then_with(|| a.node.cmp(&b.node))
    });
    if eligible.is_empty() {
        return Err("no healthy replica at required snapshot");
    }
    let mut seen_zones = HashSet::new();
    let mut used = HashSet::new();
    let mut attempts = Vec::new();
    for index in 0..eligible.len().min(max_attempts) {
        let Some(at_ms) = hedge_after_ms.checked_mul(index as u64) else { break; };
        if at_ms >= deadline_ms { break; }
        // Use the fastest unseen zone while it exists. Once each zone is
        // represented, pick the fastest remaining replica.
        let pick = eligible.iter()
            .find(|candidate| !used.contains(candidate.node.as_str())
                && !seen_zones.contains(candidate.zone.as_str()))
            .or_else(|| eligible.iter().find(|candidate| !used.contains(candidate.node.as_str())))
            .expect("at least one candidate remains");
        used.insert(pick.node.as_str());
        seen_zones.insert(pick.zone.as_str());
        attempts.push(HedgedAttempt { node: pick.node.clone(), at_ms });
    }
    Ok(attempts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(fingerprint: u128) -> SnapshotVersion {
        SnapshotVersion { epoch: 1, generation: 8, fingerprint }
    }
    fn candidate(node: &str, zone: &str, ms: u64, hash: u128) -> HedgedReplica {
        HedgedReplica {
            node: node.into(), zone: zone.into(), healthy: true,
            p95_latency_ms: ms, snapshot: version(hash)
        }
    }

    #[test]
    fn prefers_independent_zones_before_repeating_one() {
        let replicas = [
            candidate("a", "east", 10, 9),
            candidate("b", "east", 11, 9),
            candidate("c", "west", 15, 9),
        ];
        let plan = plan_hedged_reads(&replicas, version(9), 100, 20, 3).unwrap();
        assert_eq!(plan, [
            HedgedAttempt { node: "a".into(), at_ms: 0 },
            HedgedAttempt { node: "c".into(), at_ms: 20 },
            HedgedAttempt { node: "b".into(), at_ms: 40 },
        ]);
    }

    #[test]
    fn deadline_truncates_hedges() {
        let replicas = [candidate("a", "east", 10, 9), candidate("b", "west", 12, 9)];
        assert_eq!(plan_hedged_reads(&replicas, version(9), 20, 20, 2).unwrap().len(), 1);
    }

    #[test]
    fn conflicting_fingerprint_rejected_even_if_other_peer_matches() {
        let replicas = [candidate("a", "east", 10, 9), candidate("b", "west", 10, 10)];
        assert!(plan_hedged_reads(&replicas, version(9), 100, 10, 2).is_err());
    }
}

#[cfg(test)]
mod hedged_replica_identity_tests {
    use super::*;

    #[test]
    fn rejects_control_characters_and_path_like_replica_labels() {
        let version = SnapshotVersion { epoch: 1, generation: 1, fingerprint: 7 };
        for (node, zone) in [
            ("node\nother", "zone"),
            ("node/other", "zone"),
            ("node", "../zone"),
            ("node", "zone\tother"),
            ("node", "z\u{00e9}ne"),
        ] {
            let replicas = [HedgedReplica {
                node: node.into(),
                zone: zone.into(),
                healthy: true,
                p95_latency_ms: 10,
                snapshot: version,
            }];
            assert!(plan_hedged_reads(&replicas, version, 50, 10, 1).is_err());
        }
        let valid = [HedgedReplica {
            node: "node_01".into(),
            zone: "eu-central-1".into(),
            healthy: true,
            p95_latency_ms: 10,
            snapshot: version,
        }];
        assert_eq!(plan_hedged_reads(&valid, version, 50, 10, 1).unwrap().len(), 1);
    }
}
