//! Strict, explicit membership file for experimental storage clusters.
use std::fmt::Write;

const MAX_MEMBER_RECORDS: usize = 64;
const MAX_MANIFEST_BYTES: usize = 8192;
const MAX_MEMBER_WEIGHT: u16 = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub node: String,
    pub zone: String,
    pub weight: u16,
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_member_identity(node: &str, zone: &str) -> bool {
    identifier(node) && identifier(zone)
}

fn valid_member_weight(weight: u16) -> bool {
    (1..=MAX_MEMBER_WEIGHT).contains(&weight)
}

fn parse_member(line: &str) -> Result<(&str, &str, u16), &'static str> {
    if line.is_empty() {
        return Err("empty membership record");
    }
    // The wire manifest has ASCII fields and separators. Unicode spaces must
    // not silently alias valid, canonical member records.
    let mut fields = line.split_ascii_whitespace();
    let (Some(node), Some(zone), Some(weight_text), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err("invalid membership record");
    };
    if !valid_member_identity(node, zone) {
        return Err("invalid membership record");
    }
    // Reject alternate spellings such as +1 or 001. Membership manifests
    // should have one canonical byte representation per numeric weight.
    if weight_text.starts_with('0') || !weight_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("noncanonical member weight");
    }
    let weight = weight_text
        .parse::<u16>()
        .map_err(|_| "invalid member weight")?;
    if !valid_member_weight(weight) {
        return Err("duplicate member or invalid weight");
    }
    Ok((node, zone, weight))
}

/// Format: one `node zone weight` record per line, at most 64 records.
/// Membership is a configuration snapshot, not automatic discovery or consensus.
pub fn parse_membership(text: &str) -> Result<Vec<Member>, &'static str> {
    if text.len() > MAX_MANIFEST_BYTES {
        return Err("membership manifest too large");
    }
    let mut members = Vec::new();
    // Keep borrowed names in a sorted stack index. Duplicate detection is
    // logarithmic per record and never needs a temporary hash allocation.
    let mut seen = [""; MAX_MEMBER_RECORDS];
    for line in text.lines() {
        // Validate and deduplicate borrowed tokens before allocating output.
        let (node, zone, weight) = parse_member(line)?;
        let position = match seen[..members.len()].binary_search(&node) {
            Ok(_) => return Err("duplicate member or invalid weight"),
            Err(position) => position,
        };
        if members.len() >= MAX_MEMBER_RECORDS {
            return Err("too many cluster members");
        }
        seen.copy_within(position..members.len(), position + 1);
        seen[position] = node;
        members.push(Member {
            node: node.to_owned(),
            zone: zone.to_owned(),
            weight,
        });
    }
    if members.is_empty() {
        return Err("membership is empty");
    }
    Ok(members)
}

/// Validate membership and its exact serialized length without allocating
/// the manifest. Used by placement and capacity calculations.
fn serialized_membership_bytes(members: &[Member]) -> Result<usize, &'static str> {
    if members.is_empty() || members.len() > MAX_MEMBER_RECORDS {
        return Err("invalid membership record count");
    }
    let mut seen = [""; MAX_MEMBER_RECORDS];
    let mut bytes = 0usize;
    for (index, member) in members.iter().enumerate() {
        if !valid_member_identity(&member.node, &member.zone) || !valid_member_weight(member.weight)
        {
            return Err("invalid or duplicate membership record");
        }
        let position = match seen[..index].binary_search(&member.node.as_str()) {
            Ok(_) => return Err("invalid or duplicate membership record"),
            Err(position) => position,
        };
        seen.copy_within(position..index, position + 1);
        seen[position] = member.node.as_str();
        // Two spaces and one newline. Weight is always a positive u16.
        bytes += member.node.len() + member.zone.len() + member.weight.ilog10() as usize + 1 + 3;
        if bytes > MAX_MANIFEST_BYTES {
            return Err("membership manifest too large");
        }
    }
    Ok(bytes)
}

pub(crate) fn validate_membership(members: &[Member]) -> Result<(), &'static str> {
    serialized_membership_bytes(members).map(|_| ())
}

/// Stable on-disk representation of an already validated member set.
pub fn format_membership(members: &[Member]) -> Result<String, &'static str> {
    let wire_bytes = serialized_membership_bytes(members)?;
    let mut text = String::with_capacity(wire_bytes);
    for member in members {
        writeln!(
            &mut text,
            "{} {} {}",
            member.node, member.zone, member.weight
        )
        .expect("writing to String is infallible");
    }
    debug_assert_eq!(text.len(), wire_bytes);
    Ok(text)
}

/// A validated change to an explicit cluster membership configuration.
/// These are planning records only and do not mutate running replicas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MembershipChange {
    Added(Member),
    Removed(Member),
    Reconfigured { before: Member, after: Member },
}

/// Compare two membership snapshots in canonical node-name order.
/// Treat zone or weight changes as reconfigurations, and do not report a
/// change merely because the manifest listed the same nodes differently.
/// Only changed members are cloned into the output.
pub fn membership_changes(
    before: &[Member],
    after: &[Member],
) -> Result<Vec<MembershipChange>, &'static str> {
    validate_membership(before)?;
    validate_membership(after)?;
    // Membership is capped at 64 nodes. Sort borrowed pointers in fixed
    // stack slots instead of allocating two temporary Vecs for every diff.
    let mut old = [None; MAX_MEMBER_RECORDS];
    let mut new = [None; MAX_MEMBER_RECORDS];
    for (slot, member) in old.iter_mut().zip(before) {
        *slot = Some(member);
    }
    for (slot, member) in new.iter_mut().zip(after) {
        *slot = Some(member);
    }
    let old = &mut old[..before.len()];
    let new = &mut new[..after.len()];
    old.sort_unstable_by_key(|entry| entry.map(|member| member.node.as_str()));
    new.sort_unstable_by_key(|entry| entry.map(|member| member.node.as_str()));

    let mut changes = Vec::new();
    let (mut left, mut right) = (0usize, 0usize);
    while left < old.len() || right < new.len() {
        match (
            old.get(left).copied().flatten(),
            new.get(right).copied().flatten(),
        ) {
            (Some(a), Some(b)) => match a.node.cmp(&b.node) {
                std::cmp::Ordering::Less => {
                    changes.push(MembershipChange::Removed((*a).clone()));
                    left += 1;
                }
                std::cmp::Ordering::Greater => {
                    changes.push(MembershipChange::Added((*b).clone()));
                    right += 1;
                }
                std::cmp::Ordering::Equal => {
                    if a.zone != b.zone || a.weight != b.weight {
                        changes.push(MembershipChange::Reconfigured {
                            before: (*a).clone(),
                            after: (*b).clone(),
                        });
                    }
                    left += 1;
                    right += 1;
                }
            },
            (Some(a), None) => {
                changes.push(MembershipChange::Removed((*a).clone()));
                left += 1;
            }
            (None, Some(b)) => {
                changes.push(MembershipChange::Added((*b).clone()));
                right += 1;
            }
            (None, None) => break,
        }
    }
    Ok(changes)
}

/// Apply an explicit, previously reviewed membership delta as one value change.
///
/// Every removal or reconfiguration must match the exact old record, not just
/// the node name. This prevents stale control-plane deltas from silently
/// changing a newer membership snapshot. The input is never mutated.
pub fn apply_membership_changes(
    before: &[Member],
    changes: &[MembershipChange],
) -> Result<Vec<Member>, &'static str> {
    validate_membership(before)?;
    if changes.len() > 2 * MAX_MEMBER_RECORDS {
        return Err("membership delta exceeds record limit");
    }
    let mut result = before.to_vec();
    let mut touched = std::collections::HashSet::with_capacity(changes.len());
    for change in changes {
        let node = match change {
            MembershipChange::Added(member) | MembershipChange::Removed(member) => {
                member.node.as_str()
            }
            MembershipChange::Reconfigured { before, after } => {
                if before.node != after.node {
                    return Err("membership reconfiguration changed node identity");
                }
                before.node.as_str()
            }
        };
        if !touched.insert(node) {
            return Err("membership delta changes the same node twice");
        }
        let index = result.iter().position(|member| member.node == node);
        match change {
            MembershipChange::Added(member) => {
                if index.is_some() {
                    return Err("membership delta adds an existing node");
                }
                result.push(member.clone());
            }
            MembershipChange::Removed(member) => {
                let position = index.ok_or("membership delta removes an unknown node")?;
                if result[position] != *member {
                    return Err("membership delta has a stale removal");
                }
                result.swap_remove(position);
            }
            MembershipChange::Reconfigured { before, after } => {
                let position = index.ok_or("membership delta reconfigures an unknown node")?;
                if result[position] != *before {
                    return Err("membership delta has a stale reconfiguration");
                }
                result[position] = after.clone();
            }
        }
    }
    // Apply all additions and removals before validating the final snapshot.
    // A full 64-node membership can be replaced without a false intermediate
    // capacity rejection when additions precede removals.
    result.sort_unstable_by(|left, right| left.node.cmp(&right.node));
    validate_membership(&result)?;
    Ok(result)
}

#[cfg(test)]
#[path = "distributed_membership_tests/apply_membership_delta_tests.rs"]
mod apply_membership_delta_tests;

/// Capacity and node count advertised for one failure domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneCapacity {
    pub zone: String,
    pub nodes: usize,
    pub weight: u64,
}

/// Aggregate valid membership weights by zone for placement planning.
pub fn membership_capacity_by_zone(members: &[Member]) -> Result<Vec<ZoneCapacity>, &'static str> {
    validate_membership(members)?;
    // Most clusters use far fewer zones than members. Aggregate borrowed
    // zone names without a tree allocation for the small common case.
    if members.len() <= 16 {
        let mut zones = Vec::<(&str, usize, u64)>::new();
        for member in members {
            if let Some((_, nodes, weight)) =
                zones.iter_mut().find(|(zone, _, _)| *zone == member.zone)
            {
                *nodes += 1;
                *weight += u64::from(member.weight);
            } else {
                zones.push((&member.zone, 1, u64::from(member.weight)));
            }
        }
        // Preserve the public method's stable alphabetical zone ordering.
        zones.sort_unstable_by(|left, right| left.0.cmp(right.0));
        return Ok(zones
            .into_iter()
            .map(|(zone, nodes, weight)| ZoneCapacity {
                zone: zone.to_owned(),
                nodes,
                weight,
            })
            .collect());
    }

    // Membership is capped at 64 nodes. Sorting borrowed zone/weight pairs
    // on the stack replaces up to 64 BTreeMap node allocations and avoids
    // cloning zone names until a complete output group is known.
    let mut ordered = [("", 0u16); MAX_MEMBER_RECORDS];
    for (slot, member) in ordered.iter_mut().zip(members) {
        *slot = (member.zone.as_str(), member.weight);
    }
    let ordered = &mut ordered[..members.len()];
    ordered.sort_unstable_by(|left, right| left.0.cmp(right.0));
    let mut result = Vec::<ZoneCapacity>::new();
    for &(zone, weight) in ordered.iter() {
        if let Some(previous) = result.last_mut() {
            if previous.zone == zone {
                previous.nodes += 1;
                previous.weight += u64::from(weight);
                continue;
            }
        }
        result.push(ZoneCapacity {
            zone: zone.to_owned(),
            nodes: 1,
            weight: u64::from(weight),
        });
    }
    Ok(result)
}

#[cfg(test)]
#[path = "distributed_membership_tests/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "distributed_membership_tests/zone_capacity_tests.rs"]
mod zone_capacity_tests;

#[cfg(test)]
#[path = "distributed_membership_tests/small_membership_validation_tests.rs"]
mod small_membership_validation_tests;

#[cfg(test)]
#[path = "distributed_membership_tests/borrowed_member_parsing_tests.rs"]
mod borrowed_member_parsing_tests;

#[cfg(test)]
#[path = "distributed_membership_tests/adaptive_membership_parse_tests.rs"]
mod adaptive_membership_parse_tests;

#[cfg(test)]
#[path = "distributed_membership_tests/exact_membership_format_size_tests.rs"]
mod exact_membership_format_size_tests;

#[cfg(test)]
#[path = "distributed_membership_tests/canonical_membership_weight_tests.rs"]
mod canonical_membership_weight_tests;

#[cfg(test)]
#[path = "distributed_membership_tests/ascii_membership_delimiter_tests.rs"]
mod ascii_membership_delimiter_tests;

#[cfg(test)]
#[path = "distributed_membership_tests/small_zone_capacity_regressions.rs"]
mod small_zone_capacity_regressions;

#[cfg(test)]
#[path = "distributed_membership_tests/membership_reconciliation_tests.rs"]
mod membership_reconciliation_tests;

#[cfg(test)]
#[path = "distributed_membership_tests/stack_membership_index_tests.rs"]
mod stack_membership_index_tests;

#[cfg(test)]
#[path = "distributed_membership_tests/borrowed_membership_diff_tests.rs"]
mod borrowed_membership_diff_tests;

/// Canonicalize a validated membership snapshot by node name.
///
/// Unlike format_membership, this produces identical bytes regardless of
/// configuration input order. Callers may fingerprint these bytes to compare
/// snapshots, but a matching fingerprint is not consensus authorization.
pub fn canonical_membership_manifest(members: &[Member]) -> Result<String, &'static str> {
    let bytes = serialized_membership_bytes(members)?;
    let mut sorted = members.iter().collect::<Vec<_>>();
    sorted.sort_unstable_by(|left, right| left.node.cmp(&right.node));
    let mut output = String::with_capacity(bytes);
    for member in sorted {
        writeln!(
            &mut output,
            "{} {} {}",
            member.node, member.zone, member.weight
        )
        .expect("writing to String cannot fail");
    }
    debug_assert_eq!(output.len(), bytes);
    Ok(output)
}

#[cfg(test)]
#[path = "distributed_membership_tests/canonical_membership_snapshot_tests.rs"]
mod canonical_membership_snapshot_tests;

#[cfg(test)]
#[path = "distributed_membership_tests/stack_zone_capacity_tests.rs"]
mod stack_zone_capacity_tests;
