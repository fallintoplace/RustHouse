//! Rank candidate storage replicas for reads without claiming consistency.

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

fn valid_replica_version(replica: &ReplicaStatus) -> bool {
    valid_node(&replica.node) && replica.epoch != 0 && replica.generation != 0
}

fn newest_first(left: &ReplicaStatus, right: &ReplicaStatus) -> std::cmp::Ordering {
    right
        .epoch
        .cmp(&left.epoch)
        .then_with(|| right.generation.cmp(&left.generation))
        .then_with(|| left.node.cmp(&right.node))
}

fn validate_replica_nodes(replicas: &[ReplicaStatus]) -> Result<(), &'static str> {
    // All callers enforce a maximum of 64 replicas. Sorting borrowed IDs
    // on the stack avoids allocating a hash table for every read request.
    let mut nodes = [""; 64];
    for (index, replica) in replicas.iter().enumerate() {
        if !valid_replica_version(replica) {
            return Err("invalid or duplicate replica version or node");
        }
        nodes[index] = replica.node.as_str();
    }
    let sorted = &mut nodes[..replicas.len()];
    sorted.sort_unstable();
    if sorted.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("invalid or duplicate replica version or node");
    }
    Ok(())
}

fn ranked_node_ids<'a>(replicas: impl Iterator<Item = &'a ReplicaStatus>) -> Vec<String> {
    // Up to 64 validated replicas. Keep borrowed references inline; only
    // the public result allocates owned node IDs.
    let mut ranked = [None; 64];
    let mut count = 0;
    for replica in replicas {
        ranked[count] = Some(replica);
        count += 1;
    }
    let ranked = &mut ranked[..count];
    ranked.sort_unstable_by(|left, right| {
        newest_first(
            left.expect("initialized replica ranking"),
            right.expect("initialized replica ranking"),
        )
    });
    ranked
        .iter()
        .map(|item| item.expect("initialized replica ranking").node.clone())
        .collect()
}

/// Return healthy node IDs, newest advertised version first.
/// A caller must still validate returned data and enforce its consistency model.
pub fn rank_read_replicas(replicas: &[ReplicaStatus]) -> Result<Vec<String>, &'static str> {
    if replicas.is_empty() || replicas.len() > 64 {
        return Err("invalid replica status set");
    }
    validate_replica_nodes(replicas)?;
    let ranked = ranked_node_ids(replicas.iter().filter(|replica| replica.healthy));
    if ranked.is_empty() {
        return Err("no healthy storage replica");
    }
    Ok(ranked)
}

/// Rank healthy replicas at or above a required snapshot version.
/// A different epoch is a different lineage and is never considered fresh.
pub fn rank_read_replicas_at_least(
    replicas: &[ReplicaStatus],
    minimum_epoch: u64,
    minimum_generation: u64,
) -> Result<Vec<String>, &'static str> {
    if replicas.is_empty() || replicas.len() > 64 {
        return Err("invalid replica status set");
    }
    validate_replica_nodes(replicas)?;
    if minimum_epoch == 0 || minimum_generation == 0 {
        return Err("invalid replica freshness floor");
    }
    let ranked = ranked_node_ids(replicas.iter().filter(|replica| {
        replica.healthy
            && replica.epoch == minimum_epoch
            && replica.generation >= minimum_generation
    }));
    if ranked.is_empty() {
        return Err(if replicas.iter().any(|replica| replica.healthy) {
            "no storage replica meets the freshness floor"
        } else {
            "no healthy storage replica"
        });
    }
    Ok(ranked)
}

/// Select a bounded newest-first prefix without sorting every eligible node.
/// This is a hint for read failover order, not an authorization to read stale data.
pub fn rank_read_replicas_top_k(
    replicas: &[ReplicaStatus],
    max_results: usize,
) -> Result<Vec<String>, &'static str> {
    if replicas.is_empty() || replicas.len() > 64 || max_results == 0 || max_results > 64 {
        return Err("invalid replica read ranking limit");
    }
    validate_replica_nodes(replicas)?;
    let ranked = ranked_node_prefix(
        replicas.iter().filter(|replica| replica.healthy),
        max_results,
    );
    if ranked.is_empty() {
        return Err("no healthy storage replica");
    }
    Ok(ranked)
}

/// Select up to max_results healthy replicas in exactly one required epoch.
pub fn rank_read_replicas_at_least_top_k(
    replicas: &[ReplicaStatus],
    minimum_epoch: u64,
    minimum_generation: u64,
    max_results: usize,
) -> Result<Vec<String>, &'static str> {
    if replicas.is_empty() || replicas.len() > 64 || max_results == 0 || max_results > 64 {
        return Err("invalid replica read ranking limit");
    }
    validate_replica_nodes(replicas)?;
    if minimum_epoch == 0 || minimum_generation == 0 {
        return Err("invalid replica freshness floor");
    }
    let ranked = ranked_node_prefix(
        replicas.iter().filter(|replica| {
            replica.healthy
                && replica.epoch == minimum_epoch
                && replica.generation >= minimum_generation
        }),
        max_results,
    );
    if ranked.is_empty() {
        return Err(if replicas.iter().any(|replica| replica.healthy) {
            "no storage replica meets the freshness floor"
        } else {
            "no healthy storage replica"
        });
    }
    Ok(ranked)
}

fn ranked_node_prefix<'a>(
    replicas: impl Iterator<Item = &'a ReplicaStatus>,
    limit: usize,
) -> Vec<String> {
    if limit > 8 {
        let mut ranked = ranked_node_ids(replicas);
        ranked.truncate(limit);
        return ranked;
    }
    // A failover request usually needs only 1-3 ranked nodes. Keep its
    // candidates on the stack so a cluster of 64 does not need a sorted Vec.
    let mut best: [Option<&ReplicaStatus>; 8] = [None; 8];
    let mut count = 0usize;
    for candidate in replicas {
        let position = (0..count)
            .find(|&index| newest_first(candidate, best[index].expect("ranked entry")).is_lt())
            .unwrap_or(count);
        if position >= limit {
            continue;
        }
        if count < limit {
            count += 1;
        }
        for index in (position + 1..count).rev() {
            best[index] = best[index - 1];
        }
        best[position] = Some(candidate);
    }
    best[..count]
        .iter()
        .map(|replica| replica.expect("ranked entry").node.clone())
        .collect()
}

/// Select only the preferred healthy replica in one scan.
/// This avoids sorting and cloning an entire replica list for point reads.
pub fn best_read_replica(replicas: &[ReplicaStatus]) -> Result<String, &'static str> {
    if replicas.is_empty() || replicas.len() > 64 {
        return Err("invalid replica status set");
    }
    validate_replica_nodes(replicas)?;
    replicas
        .iter()
        .filter(|replica| replica.healthy)
        .min_by(|left, right| newest_first(left, right))
        .map(|replica| replica.node.clone())
        .ok_or("no healthy storage replica")
}

/// Select one healthy replica at or above a verified freshness floor.
pub fn best_read_replica_at_least(
    replicas: &[ReplicaStatus],
    minimum_epoch: u64,
    minimum_generation: u64,
) -> Result<String, &'static str> {
    if replicas.is_empty() || replicas.len() > 64 {
        return Err("invalid replica status set");
    }
    validate_replica_nodes(replicas)?;
    if minimum_epoch == 0 || minimum_generation == 0 {
        return Err("invalid replica freshness floor");
    }
    replicas
        .iter()
        .filter(|replica| {
            replica.healthy
                && replica.epoch == minimum_epoch
                && replica.generation >= minimum_generation
        })
        .min_by(|left, right| newest_first(left, right))
        .map(|replica| replica.node.clone())
        .ok_or_else(|| {
            if replicas.iter().any(|replica| replica.healthy) {
                "no storage replica meets the freshness floor"
            } else {
                "no healthy storage replica"
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_replica_validation_rejects_duplicates_and_invalid_versions() {
        for count in [1usize, 2, 8, 9, 64] {
            let mut replicas = (0..count)
                .map(|index| ReplicaStatus {
                    node: format!("n{index}"),
                    epoch: 1,
                    generation: (index + 1) as u64,
                    healthy: true,
                })
                .collect::<Vec<_>>();
            assert_eq!(rank_read_replicas(&replicas).unwrap().len(), count);
            if count > 1 {
                replicas[count - 1].node = replicas[0].node.clone();
                assert!(rank_read_replicas(&replicas).is_err());
            }
        }
    }

    #[test]
    fn direct_selection_matches_sorted_replica_rankings() {
        let replicas = (0..64)
            .map(|index| ReplicaStatus {
                node: format!("node_{index:02}"),
                epoch: 1 + (index % 3) as u64,
                generation: 1 + (index % 11) as u64,
                healthy: index % 7 != 0,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            best_read_replica(&replicas).unwrap(),
            rank_read_replicas(&replicas).unwrap()[0],
        );
        for epoch in 1..=4 {
            for generation in [1, 5, 11, 12] {
                assert_eq!(
                    best_read_replica_at_least(&replicas, epoch, generation).ok(),
                    rank_read_replicas_at_least(&replicas, epoch, generation)
                        .ok()
                        .and_then(|ranked| ranked.into_iter().next()),
                );
            }
        }
        let mut duplicates = replicas.clone();
        duplicates[1].node = duplicates[0].node.clone();
        assert!(best_read_replica(&duplicates).is_err());
        assert!(best_read_replica_at_least(&duplicates, 1, 1).is_err());
        assert!(best_read_replica(&[]).is_err());
    }

    #[test]
    fn prefers_newest_healthy_replica() {
        let replicas = vec![
            ReplicaStatus {
                node: "a".into(),
                epoch: 3,
                generation: 10,
                healthy: false,
            },
            ReplicaStatus {
                node: "b".into(),
                epoch: 3,
                generation: 9,
                healthy: true,
            },
            ReplicaStatus {
                node: "c".into(),
                epoch: 4,
                generation: 1,
                healthy: true,
            },
        ];
        assert_eq!(rank_read_replicas(&replicas).unwrap(), vec!["c", "b"]);
    }

    #[test]
    fn rejects_missing_or_duplicate_nodes() {
        let one = ReplicaStatus {
            node: "a".into(),
            epoch: 1,
            generation: 1,
            healthy: false,
        };
        assert!(rank_read_replicas(std::slice::from_ref(&one)).is_err());
        assert!(rank_read_replicas(&[one.clone(), one]).is_err());
    }
    #[test]
    fn ties_sort_by_node_and_unhealthy_nodes_cannot_win() {
        let replicas = vec![
            ReplicaStatus {
                node: "b".into(),
                epoch: 2,
                generation: 3,
                healthy: true,
            },
            ReplicaStatus {
                node: "a".into(),
                epoch: 2,
                generation: 3,
                healthy: true,
            },
            ReplicaStatus {
                node: "old".into(),
                epoch: 1,
                generation: 99,
                healthy: true,
            },
            ReplicaStatus {
                node: "new".into(),
                epoch: 3,
                generation: 9,
                healthy: false,
            },
        ];
        assert_eq!(
            rank_read_replicas(&replicas).unwrap(),
            vec!["a", "b", "old"]
        );
        assert!(
            rank_read_replicas(&[
                ReplicaStatus {
                    node: "../bad".into(),
                    epoch: 1,
                    generation: 1,
                    healthy: false
                },
                replicas[0].clone(),
            ])
            .is_err()
        );
    }
}

#[cfg(test)]
mod freshness_floor_tests {
    use super::*;

    #[test]
    fn rejects_stale_read_replicas() {
        let replicas = [
            ReplicaStatus {
                node: "old".into(),
                epoch: 4,
                generation: 9,
                healthy: true,
            },
            ReplicaStatus {
                node: "new".into(),
                epoch: 4,
                generation: 10,
                healthy: true,
            },
            ReplicaStatus {
                node: "next".into(),
                epoch: 5,
                generation: 1,
                healthy: true,
            },
        ];
        assert_eq!(
            rank_read_replicas_at_least(&replicas, 4, 10).unwrap(),
            vec!["new"]
        );
        assert!(rank_read_replicas_at_least(&replicas, 5, 2).is_err());
        assert!(rank_read_replicas_at_least(&replicas, 6, 0).is_err());
    }
}

#[cfg(test)]
mod zero_version_replica_tests {
    use super::*;

    #[test]
    fn does_not_read_from_uninitialized_replica_versions() {
        let base = ReplicaStatus {
            node: "node_a".into(),
            epoch: 1,
            generation: 1,
            healthy: true,
        };
        assert!(rank_read_replicas(&[base.clone()]).is_ok());
        assert!(
            rank_read_replicas(&[ReplicaStatus {
                epoch: 0,
                ..base.clone()
            }])
            .is_err()
        );
        assert!(
            rank_read_replicas(&[ReplicaStatus {
                generation: 0,
                ..base
            }])
            .is_err()
        );
    }
}

#[cfg(test)]
mod freshness_floor_validation_tests {
    use super::*;

    #[test]
    fn rejects_zero_version_read_floors_for_both_selection_paths() {
        let candidates = [ReplicaStatus {
            node: "node_a".into(),
            epoch: 1,
            generation: 2,
            healthy: true,
        }];
        for (epoch, generation) in [(0, 1), (1, 0), (0, 0)] {
            assert!(rank_read_replicas_at_least(&candidates, epoch, generation).is_err());
            assert!(best_read_replica_at_least(&candidates, epoch, generation).is_err());
        }
        assert_eq!(
            rank_read_replicas_at_least(&candidates, 1, 1).unwrap(),
            vec!["node_a"]
        );
        assert_eq!(
            best_read_replica_at_least(&candidates, 1, 1).unwrap(),
            "node_a"
        );
    }
}

/**
 * Replica metadata used to avoid selecting an inconsistent snapshot during
 * read planning. An observation is not proof of consensus or data integrity.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaSnapshotStatus {
    pub node: String,
    pub snapshot: crate::distributed_lineage::SnapshotVersion,
    pub healthy: bool,
}

/// Find a quorum-observed generation, then rank only healthy peers serving
/// that exact epoch, generation and fingerprint. Unlike comparing generation
/// numbers alone, a minority's newer snapshot cannot displace the quorum.
///
/// This is a pure planning helper. Callers must still verify all returned
/// replica data, and separately establish valid snapshot ancestry.
pub fn rank_quorum_snapshot_replicas(
    members: &[&str],
    policy: crate::distributed_quorum::QuorumPolicy,
    replicas: &[ReplicaSnapshotStatus],
) -> Result<(crate::distributed_lineage::SnapshotVersion, Vec<String>), &'static str> {
    policy.observe(members, &[])?;
    if replicas.len() > members.len() {
        return Err("too many replica snapshot observations");
    }
    let mut seen = 0u16;
    let mut acknowledgements = Vec::with_capacity(replicas.len());
    for replica in replicas {
        let index = members
            .iter()
            .position(|&member| member == replica.node.as_str())
            .ok_or("snapshot from unknown replica")?;
        let bit = 1u16 << index;
        if seen & bit != 0 {
            return Err("duplicate replica snapshot observation");
        }
        seen |= bit;
        if replica.snapshot.epoch == 0 || replica.snapshot.generation == 0 {
            return Err("invalid replica snapshot version");
        }
        if replica.healthy {
            acknowledgements.push((replica.node.as_str(), replica.snapshot));
        }
    }
    let snapshot = crate::distributed_lineage::highest_quorum_observed_snapshot(
        policy,
        members,
        &acknowledgements,
    )?
    .ok_or("no quorum-observed healthy snapshot")?;
    let mut nodes = replicas
        .iter()
        .filter(|status| status.healthy && status.snapshot == snapshot)
        .map(|status| status.node.clone())
        .collect::<Vec<_>>();
    nodes.sort_unstable();
    Ok((snapshot, nodes))
}

#[cfg(test)]
mod quorum_snapshot_read_tests {
    use super::*;
    use crate::distributed_lineage::SnapshotVersion;
    use crate::distributed_quorum::QuorumPolicy;

    #[test]
    fn a_minoritys_newer_generation_cannot_redirect_quorum_reads() {
        let members = ["a", "b", "c"];
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let older = SnapshotVersion {
            epoch: 2,
            generation: 7,
            fingerprint: 40,
        };
        let newer = SnapshotVersion {
            epoch: 2,
            generation: 8,
            fingerprint: 41,
        };
        let replicas = [
            ReplicaSnapshotStatus {
                node: "c".into(),
                snapshot: newer,
                healthy: true,
            },
            ReplicaSnapshotStatus {
                node: "b".into(),
                snapshot: older,
                healthy: true,
            },
            ReplicaSnapshotStatus {
                node: "a".into(),
                snapshot: older,
                healthy: true,
            },
        ];
        assert_eq!(
            rank_quorum_snapshot_replicas(&members, policy, &replicas).unwrap(),
            (older, vec!["a".into(), "b".into()]),
        );
        let mut changed = replicas.clone();
        changed[0].snapshot = older;
        let (version, nodes) = rank_quorum_snapshot_replicas(&members, policy, &changed).unwrap();
        assert_eq!(version, older);
        assert_eq!(nodes, ["a", "b", "c"]);
    }

    #[test]
    fn rejects_conflicting_and_duplicate_advertisements() {
        let members = ["a", "b", "c"];
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        let v = SnapshotVersion {
            epoch: 2,
            generation: 7,
            fingerprint: 40,
        };
        let a = ReplicaSnapshotStatus {
            node: "a".into(),
            snapshot: v,
            healthy: true,
        };
        let b = ReplicaSnapshotStatus {
            node: "b".into(),
            snapshot: v,
            healthy: true,
        };
        assert!(rank_quorum_snapshot_replicas(&members, policy, &[a.clone()]).is_err());
        assert!(rank_quorum_snapshot_replicas(&members, policy, &[a.clone(), a.clone()]).is_err());
        assert!(
            rank_quorum_snapshot_replicas(
                &members,
                policy,
                &[
                    a.clone(),
                    ReplicaSnapshotStatus {
                        snapshot: SnapshotVersion {
                            fingerprint: 99,
                            ..v
                        },
                        ..b.clone()
                    }
                ]
            )
            .is_err()
        );
        assert!(
            rank_quorum_snapshot_replicas(
                &members,
                policy,
                &[
                    a.clone(),
                    ReplicaSnapshotStatus {
                        healthy: false,
                        ..b.clone()
                    }
                ]
            )
            .is_err()
        );
        assert!(
            rank_quorum_snapshot_replicas(
                &members,
                policy,
                &[
                    ReplicaSnapshotStatus {
                        node: "unknown".into(),
                        ..a.clone()
                    },
                    b.clone()
                ]
            )
            .is_err()
        );
        assert!(
            rank_quorum_snapshot_replicas(
                &members,
                policy,
                &[
                    ReplicaSnapshotStatus {
                        snapshot: SnapshotVersion { epoch: 0, ..v },
                        ..a
                    },
                    b,
                ]
            )
            .is_err()
        );
    }

    #[test]
    fn large_replica_group_requires_distinct_healthy_quorum() {
        let names = (0..16).map(|i| format!("node_{i}")).collect::<Vec<_>>();
        let members = names.iter().map(String::as_str).collect::<Vec<_>>();
        let policy = QuorumPolicy::new(16, 9, 9).unwrap();
        let v = SnapshotVersion {
            epoch: 1,
            generation: 11,
            fingerprint: 222,
        };
        let statuses = names
            .iter()
            .enumerate()
            .map(|(i, node)| ReplicaSnapshotStatus {
                node: node.clone(),
                snapshot: v,
                healthy: i < 9,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rank_quorum_snapshot_replicas(&members, policy, &statuses)
                .unwrap()
                .1
                .len(),
            9,
        );
        let mut less = statuses;
        less[8].healthy = false;
        assert!(rank_quorum_snapshot_replicas(&members, policy, &less).is_err());
    }
}

#[cfg(test)]
mod inline_replica_ranking_tests {
    use super::*;

    #[test]
    fn inline_and_spilled_groups_match_full_sort() {
        for count in [1, 2, 8, 9, 16, 64] {
            let statuses = (0..count)
                .map(|index| ReplicaStatus {
                    node: format!("node_{index:02}"),
                    epoch: 1 + (index % 3) as u64,
                    generation: 1 + (index % 5) as u64,
                    healthy: index % 4 != 0,
                })
                .collect::<Vec<_>>();
            let mut expected = statuses
                .iter()
                .filter(|status| status.healthy)
                .collect::<Vec<_>>();
            expected.sort_unstable_by(|a, b| newest_first(a, b));
            let expected = expected
                .iter()
                .map(|status| status.node.clone())
                .collect::<Vec<_>>();
            assert_eq!(rank_read_replicas(&statuses).unwrap_or_default(), expected);
            let expected_floor = statuses
                .iter()
                .filter(|status| status.healthy && status.epoch == 2 && status.generation >= 3)
                .map(|status| status.node.clone())
                .collect::<Vec<_>>();
            let ranked_floor = rank_read_replicas_at_least(&statuses, 2, 3).unwrap_or_default();
            assert_eq!(ranked_floor.len(), expected_floor.len());
            assert!(
                ranked_floor
                    .iter()
                    .all(|node| expected_floor.contains(node))
            );
        }
    }
}

/// Prefer an independent failure zone for each early read fallback.
///
/// The first candidate remains the freshest healthy replica. A subsequent
/// candidate from a new zone is preferred over secondaries in zones already
/// visited, even when it advertises a slightly older allowed generation.
/// This is a read-routing hint, not a consistency guarantee.
pub fn rank_zone_diverse_read_replicas_at_least(
    replicas: &[ReplicaStatus],
    zones: &[(&str, &str)],
    minimum_epoch: u64,
    minimum_generation: u64,
) -> Result<Vec<String>, &'static str> {
    // Strictly validate all metadata, including unhealthy nodes, before
    // using any zone for a read failover decision.
    let ranked = rank_read_replicas_at_least(replicas, minimum_epoch, minimum_generation)?;
    if zones.len() != replicas.len() {
        return Err("replica zone membership count mismatch");
    }
    for (index, &(node, zone)) in zones.iter().enumerate() {
        if !valid_node(node)
            || zone.is_empty()
            || zone.len() > 64
            || !zone
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            || zones[..index].iter().any(|&(prior, _)| prior == node)
            || !replicas.iter().any(|replica| replica.node == node)
        {
            return Err("invalid or duplicate replica zone metadata");
        }
    }
    // At most 64 replicas can be configured. First choose the most recent
    // eligible replica from every zone, then add in-zone fallbacks in their
    // existing deterministic freshness order.
    let mut seen_zones = std::collections::HashSet::<&str>::new();
    let mut diverse = Vec::with_capacity(ranked.len());
    let mut fallback = Vec::new();
    for node in ranked {
        let zone = zones
            .iter()
            .find(|&&(candidate, _)| candidate == node)
            .map(|&(_, zone)| zone)
            .expect("validated one-to-one zone inventory");
        if seen_zones.insert(zone) {
            diverse.push(node);
        } else {
            fallback.push(node);
        }
    }
    diverse.extend(fallback);
    Ok(diverse)
}

#[cfg(test)]
mod zone_diverse_read_ranking_tests {
    use super::*;

    fn replica(node: &str, generation: u64, healthy: bool) -> ReplicaStatus {
        ReplicaStatus {
            node: node.into(),
            epoch: 3,
            generation,
            healthy,
        }
    }

    #[test]
    fn spreads_fallbacks_over_zones_without_crossing_the_snapshot_floor() {
        let replicas = [
            replica("a", 20, true),
            replica("b", 19, true),
            replica("c", 18, true),
            replica("d", 17, true),
            replica("e", 16, false),
            replica("f", 9, true),
        ];
        let zones = [
            ("a", "east"),
            ("b", "east"),
            ("c", "west"),
            ("d", "south"),
            ("e", "north"),
            ("f", "spare"),
        ];
        assert_eq!(
            rank_zone_diverse_read_replicas_at_least(&replicas, &zones, 3, 15).unwrap(),
            ["a", "c", "d", "b"],
        );
        let mut reversed = replicas.to_vec();
        reversed.reverse();
        let mut reverse_zones = zones.to_vec();
        reverse_zones.reverse();
        assert_eq!(
            rank_zone_diverse_read_replicas_at_least(&reversed, &reverse_zones, 3, 15).unwrap(),
            ["a", "c", "d", "b"],
        );
    }

    #[test]
    fn rejects_missing_duplicate_and_invalid_zone_labels() {
        let replicas = [replica("a", 2, true), replica("b", 2, true)];
        for zones in [
            vec![("a", "east")],
            vec![("a", "east"), ("a", "west")],
            vec![("a", "east"), ("unknown", "west")],
            vec![("a", "east"), ("b", "../west")],
            vec![("a", "east"), ("b", "")],
        ] {
            assert!(rank_zone_diverse_read_replicas_at_least(&replicas, &zones, 3, 1,).is_err());
        }
        assert!(
            rank_zone_diverse_read_replicas_at_least(
                &replicas,
                &[("a", "east"), ("b", "west")],
                0,
                1,
            )
            .is_err()
        );
    }

    #[test]
    fn all_healthy_candidates_in_one_zone_preserve_default_rank() {
        let replicas = [
            replica("a", 10, true),
            replica("b", 12, true),
            replica("c", 11, true),
        ];
        let zones = [("a", "same"), ("b", "same"), ("c", "same")];
        assert_eq!(
            rank_zone_diverse_read_replicas_at_least(&replicas, &zones, 3, 1).unwrap(),
            rank_read_replicas_at_least(&replicas, 3, 1).unwrap(),
        );
    }
}

#[cfg(test)]
mod bounded_read_replica_prefix_tests {
    use super::*;

    #[test]
    fn top_k_matches_full_rankings_for_skewed_healthy_and_stale_nodes() {
        for count in [1usize, 2, 3, 8, 9, 32, 64] {
            let replicas = (0..count)
                .rev()
                .map(|i| ReplicaStatus {
                    node: format!("node{i:02}"),
                    epoch: if i % 5 == 0 { 2 } else { 3 },
                    generation: (i % 19 + 1) as u64,
                    healthy: i % 7 != 0,
                })
                .collect::<Vec<_>>();
            for limit in [1usize, 2, 3, 8, 9, 32, 64] {
                let all = rank_read_replicas(&replicas);
                let prefix = rank_read_replicas_top_k(&replicas, limit);
                assert_eq!(all.is_ok(), prefix.is_ok());
                if let (Ok(full), Ok(partial)) = (all, prefix) {
                    assert_eq!(partial.as_slice(), &full[..full.len().min(limit)]);
                }
                let full = rank_read_replicas_at_least(&replicas, 3, 7);
                let partial = rank_read_replicas_at_least_top_k(&replicas, 3, 7, limit);
                assert_eq!(full.is_ok(), partial.is_ok());
                if let (Ok(full), Ok(partial)) = (full, partial) {
                    assert_eq!(partial.as_slice(), &full[..full.len().min(limit)]);
                }
            }
        }
    }

    #[test]
    fn top_k_rejects_invalid_limits_and_duplicate_nodes() {
        let node = ReplicaStatus {
            node: "node".into(),
            epoch: 2,
            generation: 10,
            healthy: true,
        };
        assert!(rank_read_replicas_top_k(&[node.clone()], 0).is_err());
        assert!(rank_read_replicas_top_k(&[node.clone()], 65).is_err());
        assert!(rank_read_replicas_top_k(&[node.clone(), node.clone()], 1).is_err());
        assert_eq!(
            rank_read_replicas_top_k(&[node.clone()], 1).unwrap(),
            vec!["node"]
        );
    }
}

#[cfg(test)]
mod stack_read_replica_order_tests {
    use super::*;

    #[test]
    fn full_size_rank_matches_reference_with_ties_and_unhealthy_nodes() {
        for size in [1usize, 2, 8, 9, 16, 32, 64] {
            let statuses = (0..size)
                .rev()
                .map(|index| ReplicaStatus {
                    node: format!("replica_{index:02}"),
                    epoch: 1 + (index % 4) as u64,
                    generation: 1 + (index % 3) as u64,
                    healthy: index % 7 != 0,
                })
                .collect::<Vec<_>>();
            let mut expected = statuses
                .iter()
                .filter(|status| status.healthy)
                .collect::<Vec<_>>();
            expected.sort_unstable_by(|a, b| newest_first(a, b));
            let expected = expected
                .iter()
                .map(|item| item.node.clone())
                .collect::<Vec<_>>();
            assert_eq!(rank_read_replicas(&statuses).unwrap_or_default(), expected);
            if size > 1 {
                let mut duplicates = statuses.clone();
                duplicates[size - 1].node = duplicates[0].node.clone();
                assert!(rank_read_replicas(&duplicates).is_err());
                assert!(best_read_replica(&duplicates).is_err());
            }
        }
    }

    #[test]
    fn freshness_rank_uses_same_order_and_rejects_bad_versions() {
        let statuses = (0..64)
            .map(|index| ReplicaStatus {
                node: format!("node_{index:02}"),
                epoch: 4,
                generation: 1 + (index % 12) as u64,
                healthy: true,
            })
            .collect::<Vec<_>>();
        let expected = statuses
            .iter()
            .filter(|status| status.generation >= 5)
            .cloned()
            .collect::<Vec<_>>();
        let mut sorted = expected;
        sorted.sort_unstable_by(newest_first);
        let names = sorted
            .iter()
            .map(|status| status.node.clone())
            .collect::<Vec<_>>();
        assert_eq!(rank_read_replicas_at_least(&statuses, 4, 5).unwrap(), names);
        let mut invalid = statuses;
        invalid[63].epoch = 0;
        assert!(rank_read_replicas(&invalid).is_err());
    }
}
