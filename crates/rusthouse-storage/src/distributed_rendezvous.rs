//! Deterministic experimental shard-to-replica placement.
use std::collections::{HashMap, HashSet};

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
    left.0.total_cmp(&right.0).then_with(|| left.1.cmp(right.1))
}

fn validate_replica_nodes(nodes: &[String], replicas: usize) -> Result<(), &'static str> {
    if nodes.is_empty()
        || nodes.len() > MAX_PLACEMENT_NODES
        || replicas == 0
        || replicas > nodes.len()
    {
        return Err("invalid replica placement configuration");
    }
    if nodes.len() <= 8 {
        for (index, node) in nodes.iter().enumerate() {
            if !valid_node(node) || nodes[..index].contains(node) {
                return Err("invalid or duplicate replica node");
            }
        }
    } else {
        // Membership has a fixed upper bound. Sort borrowed node IDs on
        // the stack instead of allocating a hash table for every placement.
        let mut names = [""; MAX_PLACEMENT_NODES];
        for (index, node) in nodes.iter().enumerate() {
            if !valid_node(node) {
                return Err("invalid or duplicate replica node");
            }
            names[index] = node.as_str();
        }
        let sorted = &mut names[..nodes.len()];
        sorted.sort_unstable();
        if sorted.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("invalid or duplicate replica node");
        }
    }
    Ok(())
}

/// Place one shard with already validated candidate membership.
fn rank_replicas_with_seed(seed: u64, nodes: &[String], replicas: usize) -> Vec<String> {
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
        return vec![winning_node.to_owned()];
    }
    if replicas == 2 {
        let outranks = |left: (u64, &str), right: (u64, &str)| {
            left.0 > right.0 || (left.0 == right.0 && left.1 < right.1)
        };
        let mut first: Option<(u64, &str)> = None;
        let mut second: Option<(u64, &str)> = None;
        for node in nodes {
            let candidate = (score_with_seed(seed, node), node.as_str());
            if first.is_none_or(|best| outranks(candidate, best)) {
                second = first;
                first = Some(candidate);
            } else if second.is_none_or(|runner_up| outranks(candidate, runner_up)) {
                second = Some(candidate);
            }
        }
        return vec![
            first.expect("at least two validated nodes").1.to_owned(),
            second.expect("at least two validated nodes").1.to_owned(),
        ];
    }
    // Membership is capped at 64. Rank on the stack rather than allocating
    // one temporary tuple vector for every shard in a placement batch.
    let mut storage = [(0u64, ""); MAX_PLACEMENT_NODES];
    for (index, node) in nodes.iter().enumerate() {
        storage[index] = (score_with_seed(seed, node), node.as_str());
    }
    let ranked = &mut storage[..nodes.len()];
    if replicas < ranked.len() {
        ranked.select_nth_unstable_by(replicas, |left, right| {
            right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1))
        });
    }
    ranked[..replicas]
        .sort_unstable_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(right.1)));
    ranked[..replicas]
        .iter()
        .map(|(_, node)| (*node).to_owned())
        .collect()
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
    validate_replica_nodes(nodes, replicas)?;
    Ok(rank_replicas_with_seed(shard_seed(shard), nodes, replicas))
}

/// Place a bounded set of shards using one shared membership validation.
/// Each shard still has its own deterministic rendezvous seed and ranking.
pub fn replicas_for_many(
    shards: &[&str],
    nodes: &[String],
    replicas: usize,
) -> Result<Vec<Vec<String>>, &'static str> {
    if shards.len() > 4096 {
        return Err("too many shards in placement batch");
    }
    validate_replica_nodes(nodes, replicas)?;
    // Reject malformed shards before allocating or producing any placements.
    if shards.iter().any(|&shard| !valid_shard(shard)) {
        return Err("invalid shard identifier");
    }
    if shards.len() <= 16 {
        return Ok(shards
            .iter()
            .map(|&shard| rank_replicas_with_seed(shard_seed(shard), nodes, replicas))
            .collect());
    }
    // Repeated shard IDs are frequent in batched placement lookups. Keep the
    // first completed placement by input index, so duplicates clone the
    // already ranked replicas without rehashing and sorting all nodes.
    // Unique inputs still compute each ranking exactly once.
    let mut first_position = HashMap::<&str, usize>::with_capacity(shards.len());
    let mut result: Vec<Vec<String>> = Vec::with_capacity(shards.len());
    for &shard in shards {
        if let Some(&index) = first_position.get(shard) {
            result.push(result[index].clone());
        } else {
            first_position.insert(shard, result.len());
            result.push(rank_replicas_with_seed(shard_seed(shard), nodes, replicas));
        }
    }
    Ok(result)
}

#[cfg(test)]
#[path = "distributed_rendezvous_tests/repeated_shard_batch_placement_tests.rs"]
mod repeated_shard_batch_placement_tests;

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
    let mut seen = (nodes.len() > 8).then(|| HashSet::with_capacity(nodes.len()));
    let mut ranked = Vec::with_capacity(if replicas <= 2 { 0 } else { nodes.len() });
    let seed = shard_seed(shard);
    let mut winner: Option<(f64, &str)> = None;
    let mut runner_up: Option<(f64, &str)> = None;
    for (index, &(node, weight)) in nodes.iter().enumerate() {
        let duplicate = if let Some(seen) = seen.as_mut() {
            !seen.insert(node)
        } else {
            nodes[..index].iter().any(|&(previous, _)| previous == node)
        };
        if !valid_node(node) || !(1..=1024).contains(&weight) || duplicate {
            return Err("invalid weighted replica node or weight");
        }
        let uniform = hash_to_open_unit_interval(score_with_seed(seed, node));
        let candidate = (-uniform.ln() / f64::from(weight), node);
        if replicas <= 2 {
            if winner.is_none_or(|best| weighted_rank_order(&candidate, &best).is_lt()) {
                runner_up = winner;
                winner = Some(candidate);
            } else if replicas == 2
                && runner_up.is_none_or(|second| weighted_rank_order(&candidate, &second).is_lt())
            {
                runner_up = Some(candidate);
            }
        } else {
            ranked.push(candidate);
        }
    }
    if replicas <= 2 {
        let mut selected = vec![winner.expect("valid nonempty nodes").1.to_owned()];
        if replicas == 2 {
            selected.push(
                runner_up
                    .expect("at least two validated nodes")
                    .1
                    .to_owned(),
            );
        }
        return Ok(selected);
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
                priority
                    .total_cmp(&best)
                    .then_with(|| member.node.cmp(&members[best_index].node))
                    .is_lt()
            }) {
                winner = Some((priority, index));
            }
        }
        return Ok(vec![
            members[winner.expect("nonempty membership").1].node.clone(),
        ]);
    }
    if replicas == 2 {
        // Two replicas are common. Find the weighted primary and the best
        // candidate in another zone with two linear passes, no heap or sort.
        // If every member shares one zone, fall back to the next-ranked node.
        let priority = |member: &crate::distributed_membership::Member| {
            let uniform = hash_to_open_unit_interval(score_with_seed(seed, &member.node));
            -uniform.ln() / f64::from(member.weight)
        };
        let ordered = |left: &(f64, usize), right: &(f64, usize)| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| members[left.1].node.cmp(&members[right.1].node))
        };
        // Placement bounds are 64 members, so keep priority scores on the
        // stack and compute each expensive logarithm only once.
        let mut storage = [(0.0f64, 0usize); MAX_PLACEMENT_NODES];
        for (index, member) in members.iter().enumerate() {
            storage[index] = (priority(member), index);
        }
        let scores = &storage[..members.len()];
        let primary = *scores
            .iter()
            .min_by(|a, b| ordered(a, b))
            .expect("nonempty validated membership");
        let alternative = scores
            .iter()
            .copied()
            .filter(|&(_, index)| members[index].zone != members[primary.1].zone)
            .min_by(|a, b| ordered(a, b))
            .or_else(|| {
                scores
                    .iter()
                    .copied()
                    .filter(|&(_, index)| index != primary.1)
                    .min_by(|a, b| ordered(a, b))
            })
            .expect("two replicas require at least two members");
        return Ok(vec![
            members[primary.1].node.clone(),
            members[alternative.1].node.clone(),
        ]);
    }
    // Rank borrowed membership indices directly. The previous path
    // revalidated nodes, allocated owned names for every candidate, and
    // built a second hash map to recover each candidate's zone.
    let mut score_storage = [(0.0f64, 0usize); MAX_PLACEMENT_NODES];
    for (index, member) in members.iter().enumerate() {
        let uniform = hash_to_open_unit_interval(score_with_seed(seed, &member.node));
        score_storage[index] = (-uniform.ln() / f64::from(member.weight), index);
    }
    let ranked = &mut score_storage[..members.len()];
    ranked.sort_unstable_by(|(left, left_index), (right, right_index)| {
        left.total_cmp(right)
            .then_with(|| members[*left_index].node.cmp(&members[*right_index].node))
    });
    // Membership has at most 64 records. Assign compact zone IDs once from
    // borrowed, stack-sorted names. The weighted pass can then test one bit
    // per member instead of allocating and hashing a zone-name set.
    // Preserve the simpler direct comparisons for very small clusters.
    let mut zone_ids = [0usize; MAX_PLACEMENT_NODES];
    let mut seen_zones = [false; MAX_PLACEMENT_NODES];
    if members.len() > 8 {
        let mut ordered = [("", 0usize); MAX_PLACEMENT_NODES];
        for (slot, (index, member)) in ordered.iter_mut().zip(members.iter().enumerate()) {
            *slot = (member.zone.as_str(), index);
        }
        let ordered = &mut ordered[..members.len()];
        ordered.sort_unstable_by(|left, right| left.0.cmp(right.0));
        let mut last_zone = None;
        let mut distinct = 0usize;
        for &(zone, index) in ordered.iter() {
            if last_zone != Some(zone) {
                last_zone = Some(zone);
                distinct += 1;
            }
            zone_ids[index] = distinct - 1;
        }
    }
    let mut stack_zones = [0usize; 8];
    let mut stack_zone_count = 0usize;
    let mut selected = Vec::with_capacity(replicas);
    let mut first_pass = [false; MAX_PLACEMENT_NODES];
    for (position, &(_, index)) in ranked.iter().enumerate() {
        let first_in_zone = if members.len() > 8 {
            let slot = &mut seen_zones[zone_ids[index]];
            let first = !*slot;
            *slot = true;
            first
        } else if stack_zones[..stack_zone_count]
            .iter()
            .any(|&prior| members[prior].zone == members[index].zone)
        {
            false
        } else {
            stack_zones[stack_zone_count] = index;
            stack_zone_count += 1;
            true
        };
        if first_in_zone {
            first_pass[position] = true;
            selected.push(members[index].node.clone());
            if selected.len() == replicas {
                return Ok(selected);
            }
        }
    }
    for (position, &(_, index)) in ranked.iter().enumerate() {
        if !first_pass[position] {
            selected.push(members[index].node.clone());
            if selected.len() == replicas {
                break;
            }
        }
    }
    Ok(selected)
}

/// Rank only eligible replica nodes, without silently assigning a full node.
///
/// This is placement planning, not an allocation or an on-disk reservation.
/// The transfer executor must reserve capacity again before copying.
pub fn capacity_aware_replicas_for(
    shard: &str,
    members: &[crate::distributed_membership::Member],
    capacities: &[(&str, crate::distributed_capacity::Capacity)],
    object_bytes: u64,
    replicas: usize,
) -> Result<Vec<String>, &'static str> {
    crate::distributed_membership::validate_membership(members)?;
    if replicas == 0 || replicas > members.len() || capacities.len() != members.len() {
        return Err("invalid capacity-aware replica configuration");
    }
    // Keep validated full membership borrowed unless capacity filters a node.
    // Cloning eligible members is only necessary once filtering begins.
    let mut eligible: Option<Vec<crate::distributed_membership::Member>> = None;
    if capacities.len() <= 8 {
        // A typical replica group has only a few nodes. Validate every
        // capacity name before matching, while keeping the lookup borrowed
        // and allocation-free. The planner still validates every capacity
        // state and rejects missing or duplicate entries.
        for (index, &(node, _)) in capacities.iter().enumerate() {
            if capacities[..index]
                .iter()
                .any(|&(previous, _)| previous == node)
            {
                return Err("duplicate replica capacity entry");
            }
        }
        for (index, member) in members.iter().enumerate() {
            let capacity = capacities
                .iter()
                .find(|&&(node, _)| node == member.node.as_str())
                .map(|&(_, capacity)| capacity)
                .ok_or("missing replica capacity")?;
            if capacity.available()? >= object_bytes {
                if let Some(filtered) = eligible.as_mut() {
                    filtered.push(member.clone());
                }
            } else if eligible.is_none() {
                eligible = Some(members[..index].to_vec());
            }
        }
    } else {
        let mut by_node = std::collections::HashMap::with_capacity(capacities.len());
        for &(node, capacity) in capacities {
            if by_node.insert(node, capacity).is_some() {
                return Err("duplicate replica capacity entry");
            }
        }
        for (index, member) in members.iter().enumerate() {
            let capacity = by_node
                .get(member.node.as_str())
                .ok_or("missing replica capacity")?;
            if capacity.available()? >= object_bytes {
                if let Some(filtered) = eligible.as_mut() {
                    filtered.push(member.clone());
                }
            } else if eligible.is_none() {
                eligible = Some(members[..index].to_vec());
            }
        }
    }
    if let Some(filtered) = eligible {
        if filtered.len() < replicas {
            return Err("insufficient replica capacity for requested redundancy");
        }
        zone_aware_replicas_for(shard, &filtered, replicas)
    } else {
        zone_aware_replicas_for(shard, members, replicas)
    }
}

#[cfg(test)]
#[path = "distributed_rendezvous_tests/capacity_aware_replica_tests.rs"]
mod capacity_aware_replica_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/borrowed_zone_ranking_tests.rs"]
mod borrowed_zone_ranking_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/zone_aware_tests.rs"]
mod zone_aware_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/open_probability_endpoint_tests.rs"]
mod open_probability_endpoint_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/single_zone_aware_primary_tests.rs"]
mod single_zone_aware_primary_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/portable_shard_identifier_tests.rs"]
mod portable_shard_identifier_tests;

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
    // Most rebalances move only a few replicas. Search those borrowed
    // names directly instead of allocating two hash tables per shard.
    let previous_set =
        (previous.len() > 8).then(|| previous.iter().map(String::as_str).collect::<HashSet<_>>());
    let next_set =
        (next.len() > 8).then(|| next.iter().map(String::as_str).collect::<HashSet<_>>());
    let in_previous = |node: &str| match &previous_set {
        Some(set) => set.contains(node),
        None => previous.iter().any(|candidate| candidate.as_str() == node),
    };
    let in_next = |node: &str| match &next_set {
        Some(set) => set.contains(node),
        None => next.iter().any(|candidate| candidate.as_str() == node),
    };
    let incoming = next
        .iter()
        .filter(|node| !in_previous(node))
        .cloned()
        .collect::<Vec<_>>();
    let retained = next
        .iter()
        .filter(|node| in_previous(node))
        .cloned()
        .collect::<Vec<_>>();
    let outgoing = previous
        .iter()
        .filter(|node| !in_next(node))
        .cloned()
        .collect::<Vec<_>>();
    Ok(ReplicaRebalance {
        new_placement: next,
        incoming,
        retained,
        outgoing,
    })
}

/// Plan verified replica movement for a bounded group of shards.
///
/// Validate the entire input set before planning any shard. Duplicate shard
/// IDs reuse the first deterministic result, including movement ordering.
/// Incoming replicas must still be copied and verified before retiring old
/// destinations. This API only computes placement changes.
pub fn plan_zone_aware_rebalance_many(
    shards: &[&str],
    previous_members: &[crate::distributed_membership::Member],
    next_members: &[crate::distributed_membership::Member],
    replicas: usize,
) -> Result<Vec<ReplicaRebalance>, &'static str> {
    if shards.len() > 4096 {
        return Err("too many shards in rebalance batch");
    }
    crate::distributed_membership::validate_membership(previous_members)?;
    crate::distributed_membership::validate_membership(next_members)?;
    if replicas == 0 || replicas > previous_members.len() || replicas > next_members.len() {
        return Err("invalid zone-aware replica count");
    }
    if shards.iter().any(|&shard| !valid_shard(shard)) {
        return Err("invalid shard identifier");
    }
    if shards.len() <= 16 {
        return shards
            .iter()
            .map(|&shard| {
                plan_zone_aware_rebalance(shard, previous_members, next_members, replicas)
            })
            .collect();
    }
    let mut first_position = HashMap::<&str, usize>::with_capacity(shards.len());
    let mut planned = Vec::<ReplicaRebalance>::with_capacity(shards.len());
    for &shard in shards {
        if let Some(&first) = first_position.get(shard) {
            planned.push(planned[first].clone());
        } else {
            let plan = plan_zone_aware_rebalance(shard, previous_members, next_members, replicas)?;
            first_position.insert(shard, planned.len());
            planned.push(plan);
        }
    }
    Ok(planned)
}

#[cfg(test)]
#[path = "distributed_rendezvous_tests/batched_zone_rebalance_tests.rs"]
mod batched_zone_rebalance_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/replica_rebalance_tests.rs"]
mod replica_rebalance_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/two_replica_linear_selection_tests.rs"]
mod two_replica_linear_selection_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/allocation_free_membership_tests.rs"]
mod allocation_free_membership_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/small_stack_zone_selection_tests.rs"]
mod small_stack_zone_selection_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/stack_zone_ranking_parity_tests.rs"]
mod stack_zone_ranking_parity_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/two_replica_rendezvous_selection_tests.rs"]
mod two_replica_rendezvous_selection_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/hash_threshold_rebalance_tests.rs"]
mod hash_threshold_rebalance_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/inline_capacity_lookup_tests.rs"]
mod inline_capacity_lookup_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/weighted_top_two_reference_tests.rs"]
mod weighted_top_two_reference_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/batched_rendezvous_placement_tests.rs"]
mod batched_rendezvous_placement_tests;

/// A side-effect-free step of replica placement migration.
///
/// The caller must authenticate every reported completed copy before passing
/// it as verified. No output here performs I/O or authorizes a physical delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardedRebalanceStep {
    pub new_placement: Vec<String>,
    /// Destinations requiring a copy in the current bounded scheduling round.
    pub copy_to: Vec<String>,
    /// Every incoming destination that has not yet been verified.
    pub awaiting_verification: Vec<String>,
    /// Only released after all incoming destinations are verified.
    pub retire_after_verification: Vec<String>,
}

/// Compute a two-phase migration. Outgoing nodes cannot be retired while any
/// incoming node is unverified, even when the current copy batch is empty.
/// Verification records must name actual new destinations without duplicates.
pub fn plan_guarded_zone_rebalance_step(
    shard: &str,
    previous_members: &[crate::distributed_membership::Member],
    next_members: &[crate::distributed_membership::Member],
    replicas: usize,
    verified_incoming: &[&str],
    max_copy_requests: usize,
) -> Result<GuardedRebalanceStep, &'static str> {
    if max_copy_requests > 64 {
        return Err("rebalance copy batch exceeds limit");
    }
    let movement = plan_zone_aware_rebalance(shard, previous_members, next_members, replicas)?;
    if verified_incoming.len() > movement.incoming.len() {
        return Err("rebalance verification count exceeds incoming set");
    }
    for (index, &verified) in verified_incoming.iter().enumerate() {
        if !movement
            .incoming
            .iter()
            .any(|candidate| candidate == verified)
            || verified_incoming[..index].contains(&verified)
        {
            return Err("unknown or duplicate verified rebalance destination");
        }
    }

    let awaiting_verification = movement
        .incoming
        .iter()
        .filter(|node| !verified_incoming.contains(&node.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let copy_to = awaiting_verification
        .iter()
        .take(max_copy_requests)
        .cloned()
        .collect();
    let retire_after_verification = if awaiting_verification.is_empty() {
        movement.outgoing
    } else {
        Vec::new()
    };
    Ok(GuardedRebalanceStep {
        new_placement: movement.new_placement,
        copy_to,
        awaiting_verification,
        retire_after_verification,
    })
}

#[cfg(test)]
#[path = "distributed_rendezvous_tests/verified_rebalance_barrier_tests.rs"]
mod verified_rebalance_barrier_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/borrowed_capacity_placement_tests.rs"]
mod borrowed_capacity_placement_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/stack_placement_validation_tests.rs"]
mod stack_placement_validation_tests;

#[cfg(test)]
#[path = "distributed_rendezvous_tests/stack_zone_first_rank_tests.rs"]
mod stack_zone_first_rank_tests;
