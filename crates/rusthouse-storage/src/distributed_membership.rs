//! Strict, explicit membership file for experimental storage clusters.
use std::collections::HashSet;
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
    let mut fields = line.split_whitespace();
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
    if weight_text.starts_with('0')
        || !weight_text.bytes().all(|byte| byte.is_ascii_digit())
    {
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
    // Most deployments have only a few members. Avoid allocating a hash
    // table until the ninth record while still checking every duplicate.
    let mut first_eight = [""; 8];
    let mut seen: Option<HashSet<&str>> = None;
    for line in text.lines() {
        // Inspect borrowed tokens before allocating the owned output fields.
        // Invalid, duplicate and over-limit entries do not allocate Strings.
        let (node, zone, weight) = parse_member(line)?;
        let duplicate = if members.len() < first_eight.len() {
            let duplicate = first_eight[..members.len()].contains(&node);
            first_eight[members.len()] = node;
            duplicate
        } else {
            !seen
                .get_or_insert_with(|| first_eight.iter().copied().collect())
                .insert(node)
        };
        if duplicate {
            return Err("duplicate member or invalid weight");
        }
        if members.len() >= MAX_MEMBER_RECORDS {
            return Err("too many cluster members");
        }
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
    let mut seen = (members.len() > 8).then(|| HashSet::with_capacity(members.len()));
    let mut bytes = 0usize;
    for (index, member) in members.iter().enumerate() {
        let duplicate = if let Some(seen) = seen.as_mut() {
            !seen.insert(member.node.as_str())
        } else {
            members[..index].iter().any(|previous| previous.node == member.node)
        };
        if !valid_member_identity(&member.node, &member.zone)
            || !valid_member_weight(member.weight)
            || duplicate
        {
            return Err("invalid or duplicate membership record");
        }
        // Two spaces and one newline. Weight is always a positive u16.
        bytes += member.node.len() + member.zone.len()
            + member.weight.ilog10() as usize + 1 + 3;
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

/// Capacity and node count advertised for one failure domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneCapacity {
    pub zone: String,
    pub nodes: usize,
    pub weight: u64,
}

/// Aggregate valid membership weights by zone for placement planning.
pub fn membership_capacity_by_zone(
    members: &[Member],
) -> Result<Vec<ZoneCapacity>, &'static str> {
    validate_membership(members)?;
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
        assert_eq!(format_membership(&small).unwrap(), "a zone1 42\nb zone2 2\n");
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
                ZoneCapacity { zone: "zone_a".into(), nodes: 2, weight: 7 },
                ZoneCapacity { zone: "zone_b".into(), nodes: 1, weight: 2 },
            ]
        );
        assert!(membership_capacity_by_zone(&[]).is_err());
        assert!(membership_capacity_by_zone(&[Member {
            node: "bad".into(), zone: "a".into(), weight: 0
        }]).is_err());
    }
}

#[cfg(test)]
mod small_membership_validation_tests {
    use super::*;

    #[test]
    fn small_and_large_validation_reject_the_same_duplicates() {
        for count in [1, 2, 3, 8, 9, 16, 64] {
            let valid = (0..count).map(|index| Member {
                node: format!("node_{index}"), zone: format!("zone_{}", index % 3), weight: 1,
            }).collect::<Vec<_>>();
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
        assert_eq!(format_membership(&members).unwrap(), "n1 zone1 23\nn2 zone2 7\n");
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
            let members = (0..count).map(|index| Member {
                node: format!("replica_{index:02}_{}", "n".repeat(40)),
                zone: format!("zone_{}", index % 7),
                weight: [1, 9, 10, 999, 1000][index % 5],
            }).collect::<Vec<_>>();
            let size = serialized_membership_bytes(&members).unwrap();
            let formatted = format_membership(&members).unwrap();
            assert_eq!(formatted.len(), size);
            assert_eq!(parse_membership(&formatted).unwrap(), members);
        }
    }

    #[test]
    fn sizing_does_not_skip_duplicate_or_oversized_validation() {
        let item = Member { node: "n".into(), zone: "z".into(), weight: 1 };
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
