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

    // Large, zone-heavy membership snapshots keep the O(n log n) path.
    let mut zones = std::collections::BTreeMap::<&str, (usize, u64)>::new();
    for member in members {
        let entry = zones.entry(&member.zone).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += u64::from(member.weight);
    }
    Ok(zones
        .into_iter()
        .map(|(zone, (nodes, weight))| ZoneCapacity {
            zone: zone.to_owned(),
            nodes,
            weight,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_membership_owns_fields_after_input_is_dropped() {
        let members = {
            let manifest = "n1 z1 1\nn2 z2 3\n".to_owned();
            parse_membership(&manifest).unwrap()
        };
        assert_eq!(format_membership(&members).unwrap(), "n1 z1 1\nn2 z2 3\n");
        assert!(parse_membership("n1 z1 1\nn1 z2 2\n").is_err());
    }

    #[test]
    fn round_trips_membership() {
        let parsed = parse_membership("node_a eu1 3\nnode_b eu2 1\n").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            format_membership(&parsed).unwrap(),
            "node_a eu1 3\nnode_b eu2 1\n"
        );
    }

    #[test]
    fn rejects_duplicates_and_bad_records() {
        for input in [
            "",
            "a eu1 0\n",
            "a eu1 1\na eu2 2\n",
            "../etc eu 1\n",
            "a eu 1 extra\n",
        ] {
            assert!(parse_membership(input).is_err(), "{input}");
        }
    }
    #[test]
    fn validation_matches_serialized_manifest_limits() {
        let members = (0..64)
            .map(|index| Member {
                node: format!("{}{:04}", "n".repeat(60), index),
                zone: "z".repeat(64),
                weight: 1000,
            })
            .collect::<Vec<_>>();
        assert!(validate_membership(&members).is_err());
        assert!(format_membership(&members).is_err());
        let small = parse_membership("a zone1 42\nb zone2 2\n").unwrap();
        assert!(validate_membership(&small).is_ok());
        assert_eq!(
            format_membership(&small).unwrap(),
            "a zone1 42\nb zone2 2\n"
        );
    }

    #[test]
    fn formatting_rejects_oversized_and_duplicate_records() {
        let huge = Member {
            node: "x".repeat(1_000_000),
            zone: "z".into(),
            weight: 1,
        };
        assert!(format_membership(&[huge]).is_err());
        let member = Member {
            node: "n".into(),
            zone: "z".into(),
            weight: 1,
        };
        assert!(format_membership(&[member.clone(), member]).is_err());
        assert!(format_membership(&[]).is_err());
    }
}

#[cfg(test)]
mod zone_capacity_tests {
    use super::*;

    #[test]
    fn reports_stable_totals_by_failure_domain() {
        let members = parse_membership("c zone_b 2\na zone_a 4\nb zone_a 3\n").unwrap();
        assert_eq!(
            membership_capacity_by_zone(&members).unwrap(),
            vec![
                ZoneCapacity {
                    zone: "zone_a".into(),
                    nodes: 2,
                    weight: 7
                },
                ZoneCapacity {
                    zone: "zone_b".into(),
                    nodes: 1,
                    weight: 2
                },
            ]
        );
        assert!(membership_capacity_by_zone(&[]).is_err());
        assert!(
            membership_capacity_by_zone(&[Member {
                node: "bad".into(),
                zone: "a".into(),
                weight: 0
            }])
            .is_err()
        );
    }
}

#[cfg(test)]
mod small_membership_validation_tests {
    use super::*;

    #[test]
    fn small_and_large_validation_reject_the_same_duplicates() {
        for count in [1, 2, 3, 8, 9, 16, 64] {
            let valid = (0..count)
                .map(|index| Member {
                    node: format!("node_{index}"),
                    zone: format!("zone_{}", index % 3),
                    weight: 1,
                })
                .collect::<Vec<_>>();
            assert!(validate_membership(&valid).is_ok());
            if count > 1 {
                let mut invalid = valid.clone();
                invalid[count - 1].node = invalid[0].node.clone();
                assert!(validate_membership(&invalid).is_err());
            }
        }
    }
}

#[cfg(test)]
mod borrowed_member_parsing_tests {
    use super::*;

    #[test]
    fn parses_full_membership_without_duplicating_or_losing_values() {
        let manifest = (0..64)
            .map(|index| format!("n{index} zone{} {}\n", index % 8, index + 1))
            .collect::<String>();
        let parsed = parse_membership(&manifest).unwrap();
        assert_eq!(parsed.len(), 64);
        assert_eq!(format_membership(&parsed).unwrap(), manifest);
        assert!(parse_membership(&(manifest.clone() + "overflow zone0 1\n")).is_err());
        assert!(parse_membership(&(manifest + "n0 zone1 1\n")).is_err());
    }

    #[test]
    fn preserves_validation_with_irregular_whitespace() {
        let members = parse_membership("n1\tzone1\t23\nn2   zone2  7\n").unwrap();
        assert_eq!(members[0].node, "n1");
        assert_eq!(members[0].weight, 23);
        assert_eq!(members[1].zone, "zone2");
        assert_eq!(
            format_membership(&members).unwrap(),
            "n1 zone1 23\nn2 zone2 7\n"
        );
    }
}

#[cfg(test)]
mod adaptive_membership_parse_tests {
    use super::*;

    #[test]
    fn validates_duplicates_across_the_stack_to_hash_transition() {
        let mut text = String::new();
        for index in 0..8 {
            text.push_str(&format!("node{index} zone0 1\n"));
        }
        assert_eq!(parse_membership(&text).unwrap().len(), 8);
        let mut duplicate = text.clone();
        duplicate.push_str("node0 zone1 2\n");
        assert!(parse_membership(&duplicate).is_err());
        let mut valid = text;
        valid.push_str("node8 zone1 2\n");
        assert_eq!(parse_membership(&valid).unwrap().len(), 9);
        valid.push_str("node8 zone0 1\n");
        assert!(parse_membership(&valid).is_err());
    }

    #[test]
    fn full_membership_has_unique_names_at_the_record_limit() {
        let text = (0..64)
            .map(|index| format!("node{index} zone{} 1\n", index % 4))
            .collect::<String>();
        assert_eq!(parse_membership(&text).unwrap().len(), 64);
    }
}

#[cfg(test)]
mod exact_membership_format_size_tests {
    use super::*;

    #[test]
    fn validation_size_matches_wire_output_for_long_member_names() {
        for count in [1, 8, 9, 32, 64] {
            let members = (0..count)
                .map(|index| Member {
                    node: format!("replica_{index:02}_{}", "n".repeat(40)),
                    zone: format!("zone_{}", index % 7),
                    weight: [1, 9, 10, 999, 1000][index % 5],
                })
                .collect::<Vec<_>>();
            let size = serialized_membership_bytes(&members).unwrap();
            let formatted = format_membership(&members).unwrap();
            assert_eq!(formatted.len(), size);
            assert_eq!(parse_membership(&formatted).unwrap(), members);
        }
    }

    #[test]
    fn sizing_does_not_skip_duplicate_or_oversized_validation() {
        let item = Member {
            node: "n".into(),
            zone: "z".into(),
            weight: 1,
        };
        assert!(serialized_membership_bytes(&[item.clone(), item]).is_err());
        assert!(serialized_membership_bytes(&[]).is_err());
    }
}

#[cfg(test)]
mod canonical_membership_weight_tests {
    use super::*;

    #[test]
    fn rejects_ambiguous_numeric_weights_before_manifest_serialization() {
        for weight in ["+1", "01", "0001", "-1", "1.0", "0", "1001", "١"] {
            let manifest = format!("node zone {weight}\n");
            assert!(parse_membership(&manifest).is_err(), "weight={weight}");
        }
        for weight in [1, 9, 10, 100, 1000] {
            let manifest = format!("node zone {weight}\n");
            let members = parse_membership(&manifest).unwrap();
            assert_eq!(format_membership(&members).unwrap(), manifest);
        }
    }
}

#[cfg(test)]
mod ascii_membership_delimiter_tests {
    use super::*;

    #[test]
    fn rejects_unicode_whitespace_between_or_around_fields() {
        for text in [
            "node\u{00a0}zone 1\n",
            "node zone\u{2003}1\n",
            "\u{202f}node zone 1\n",
            "node zone 1\u{0085}\n",
        ] {
            assert!(parse_membership(text).is_err(), "{text:?}");
        }
        let members = parse_membership("n1\tzone\t1\r\nn2 zone 2\n").unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(
            format_membership(&members).unwrap(),
            "n1 zone 1\nn2 zone 2\n"
        );
    }
}

#[cfg(test)]
mod small_zone_capacity_regressions {
    use super::*;

    fn membership(n: usize) -> Vec<Member> {
        (0..n)
            .map(|index| Member {
                node: format!("node{index}"),
                zone: format!("z{}", index % 3),
                weight: u16::try_from(index + 1).unwrap(),
            })
            .collect()
    }

    #[test]
    fn small_and_large_membership_use_the_same_sorted_zone_totals() {
        for count in [1, 3, 8, 16, 17, 32, 64] {
            let members = membership(count);
            let zones = membership_capacity_by_zone(&members).unwrap();
            assert_eq!(zones.len(), count.min(3));
            for (index, zone) in zones.iter().enumerate() {
                assert_eq!(zone.zone, format!("z{index}"));
                let matching = members.iter().filter(|m| m.zone == zone.zone);
                assert_eq!(zone.nodes, matching.clone().count());
                assert_eq!(
                    zone.weight,
                    matching.map(|m| u64::from(m.weight)).sum::<u64>()
                );
            }
        }
    }

    #[test]
    fn invalid_membership_is_rejected_before_aggregation() {
        let mut members = membership(3);
        members[2].node = members[0].node.clone();
        assert!(membership_capacity_by_zone(&members).is_err());
    }
}

#[cfg(test)]
mod membership_reconciliation_tests {
    use super::*;

    #[test]
    fn detects_add_remove_and_reconfiguration_in_canonical_order() {
        let before = parse_membership("c zone0 1\nb zone1 2\na zone1 3\n").unwrap();
        let after = parse_membership("d zone3 1\na zone1 3\nb zone2 2\n").unwrap();
        let changes = membership_changes(&before, &after).unwrap();
        assert_eq!(
            changes,
            vec![
                MembershipChange::Reconfigured {
                    before: before[1].clone(),
                    after: after[2].clone(),
                },
                MembershipChange::Removed(before[0].clone()),
                MembershipChange::Added(after[0].clone()),
            ]
        );
        let mut reordered = before.clone();
        reordered.reverse();
        assert!(membership_changes(&before, &reordered).unwrap().is_empty());
    }

    #[test]
    fn large_manifest_only_clones_changed_members() {
        let before = (0..64)
            .map(|index| Member {
                node: format!("node_{index:03}"),
                zone: format!("zone_{}", index % 4),
                weight: 1,
            })
            .collect::<Vec<_>>();
        let mut after = before.clone();
        after.reverse();
        assert!(membership_changes(&before, &after).unwrap().is_empty());
        after[0].weight = 1000;
        after[63].zone = "new_zone".into();
        let changes = membership_changes(&before, &after).unwrap();
        assert_eq!(changes.len(), 2);
        assert!(matches!(changes[0], MembershipChange::Reconfigured { .. }));
        assert!(matches!(changes[1], MembershipChange::Reconfigured { .. }));
        let mut duplicate = before.clone();
        duplicate[1].node = duplicate[0].node.clone();
        assert!(membership_changes(&duplicate, &after).is_err());
        let mut invalid = after.clone();
        invalid[3].weight = 0;
        assert!(membership_changes(&before, &invalid).is_err());
    }
}

#[cfg(test)]
mod stack_membership_index_tests {
    use super::*;

    #[test]
    fn full_membership_roundtrips_without_losing_duplicate_detection() {
        for size in [1usize, 2, 8, 9, 16, 32, 64] {
            let records = (0..size)
                .rev()
                .map(|index| format!("node_{index:02} zone_{} {}\n", index % 7, index % 1000 + 1))
                .collect::<String>();
            let members = parse_membership(&records).unwrap();
            assert_eq!(members.len(), size);
            assert_eq!(
                serialized_membership_bytes(&members).unwrap(),
                records.len()
            );
            assert_eq!(format_membership(&members).unwrap(), records);
            let mut duplicates = members.clone();
            if size == 1 {
                let repeated = duplicates[0].clone();
                duplicates.push(repeated);
            } else {
                duplicates[size - 1].node = duplicates[0].node.clone();
            }
            assert!(serialized_membership_bytes(&duplicates).is_err());
            let mut duplicate_text = records.clone();
            duplicate_text.push_str("node_00 zone_extra 1\n");
            assert!(parse_membership(&duplicate_text).is_err());
        }
    }

    #[test]
    fn exhausted_index_rejects_new_names_and_old_duplicates() {
        let text = (0..64)
            .map(|index| format!("node_{index:02} zone_{} 1\n", index % 4))
            .collect::<String>();
        assert_eq!(parse_membership(&text).unwrap().len(), 64);
        assert!(parse_membership(&(text.clone() + "new_node zone_0 1\n")).is_err());
        assert!(parse_membership(&(text + "node_00 zone_0 1\n")).is_err());
    }
}

#[cfg(test)]
mod borrowed_membership_diff_tests {
    use super::*;

    #[test]
    fn unsorted_snapshot_diffs_keep_canonical_order_and_values() {
        let before = parse_membership("gamma z1 1\nalpha z1 1\nbeta z2 2\n").unwrap();
        let after = parse_membership("beta z2 3\ndelta z1 1\nalpha z1 1\n").unwrap();
        let changes = membership_changes(&before, &after).unwrap();
        assert_eq!(
            changes,
            vec![
                MembershipChange::Reconfigured {
                    before: before[2].clone(),
                    after: after[0].clone(),
                },
                MembershipChange::Added(after[1].clone()),
                MembershipChange::Removed(before[0].clone()),
            ]
        );

        let mut reversed_before = before.clone();
        reversed_before.reverse();
        let mut reversed_after = after.clone();
        reversed_after.reverse();
        assert_eq!(
            membership_changes(&reversed_before, &reversed_after).unwrap(),
            changes
        );
    }

    #[test]
    fn fully_identical_maximum_memberships_do_not_emit_changes() {
        let nodes = (0..64)
            .map(|index| Member {
                node: format!("node_{index:02}"),
                zone: format!("zone_{}", index % 8),
                weight: 1,
            })
            .collect::<Vec<_>>();
        let mut reverse = nodes.clone();
        reverse.reverse();
        assert!(membership_changes(&nodes, &reverse).unwrap().is_empty());
    }
}

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
mod canonical_membership_snapshot_tests {
    use super::*;

    #[test]
    fn equal_memberships_produce_identical_sorted_bytes() {
        let ordered = parse_membership("zeta west 3\nalpha east 1\ngamma east 2\n").unwrap();
        let reversed = ordered.iter().rev().cloned().collect::<Vec<_>>();
        let a = canonical_membership_manifest(&ordered).unwrap();
        let b = canonical_membership_manifest(&reversed).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, "alpha east 1\ngamma east 2\nzeta west 3\n");
        assert_eq!(parse_membership(&a).unwrap().len(), 3);
        assert_ne!(format_membership(&ordered).unwrap(), a);
        assert_eq!(
            format_membership(&reversed).unwrap(),
            "gamma east 2\nalpha east 1\nzeta west 3\n"
        );
    }

    #[test]
    fn large_snapshot_still_obeys_exact_wire_limit_and_uniqueness() {
        let nodes = (0..64)
            .rev()
            .map(|index| Member {
                node: format!("node_{index:02}"),
                zone: format!("zone_{}", index % 4),
                weight: index as u16 + 1,
            })
            .collect::<Vec<_>>();
        let canonical = canonical_membership_manifest(&nodes).unwrap();
        assert_eq!(parse_membership(&canonical).unwrap().len(), nodes.len());
        assert_eq!(
            canonical_membership_manifest(&parse_membership(&canonical).unwrap()).unwrap(),
            canonical
        );
        let mut duplicates = nodes;
        duplicates.push(duplicates[0].clone());
        assert!(canonical_membership_manifest(&duplicates).is_err());
    }
}
