//! Strict, explicit membership file for experimental storage clusters.
use std::collections::HashSet;
use std::fmt::Write;

const MAX_MEMBERS: usize = 64;
const MAX_MEMBERSHIP_BYTES: usize = 8192;
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

fn parse_member(line: &str) -> Result<Member, &'static str> {
    if line.is_empty() {
        return Err("empty membership record");
    }
    let mut fields = line.split_whitespace();
    let (Some(node), Some(zone), Some(weight_text), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err("invalid membership record");
    };
    if !identifier(node) || !identifier(zone) {
        return Err("invalid membership record");
    }
    let weight = weight_text
        .parse::<u16>()
        .map_err(|_| "invalid member weight")?;
    if !(1..=MAX_MEMBER_WEIGHT).contains(&weight) {
        return Err("duplicate member or invalid weight");
    }
    Ok(Member {
        node: node.to_owned(),
        zone: zone.to_owned(),
        weight,
    })
}

/// Format: one `node zone weight` record per line, at most 64 records.
/// Membership is a configuration snapshot, not automatic discovery or consensus.
pub fn parse_membership(text: &str) -> Result<Vec<Member>, &'static str> {
    if text.len() > MAX_MEMBERSHIP_BYTES {
        return Err("membership manifest too large");
    }
    let mut members = Vec::new();
    let mut seen = HashSet::new();
    for line in text.lines() {
        let member = parse_member(line)?;
        if !seen.insert(member.node.clone()) {
            return Err("duplicate member or invalid weight");
        }
        members.push(member);
        if members.len() > MAX_MEMBERS {
            return Err("too many cluster members");
        }
    }
    if members.is_empty() {
        return Err("membership is empty");
    }
    Ok(members)
}

/// Stable on-disk representation of an already validated member set.
pub fn format_membership(members: &[Member]) -> Result<String, &'static str> {
    if members.is_empty() || members.len() > MAX_MEMBERS {
        return Err("invalid membership record count");
    }
    let mut seen = HashSet::new();
    for member in members {
        if !identifier(&member.node)
            || !identifier(&member.zone)
            || !(1..=MAX_MEMBER_WEIGHT).contains(&member.weight)
            || !seen.insert(member.node.as_str())
        {
            return Err("invalid or duplicate membership record");
        }
    }
    let mut text = String::with_capacity((members.len() * 64).min(MAX_MEMBERSHIP_BYTES));
    for member in members {
        writeln!(
            &mut text,
            "{} {} {}",
            member.node, member.zone, member.weight
        )
        .expect("writing to String is infallible");
        if text.len() > MAX_MEMBERSHIP_BYTES {
            return Err("membership manifest too large");
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    format_membership(members)?;
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
