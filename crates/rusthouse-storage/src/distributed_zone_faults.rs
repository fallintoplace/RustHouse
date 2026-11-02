//! Exact zone-outage witnesses for weighted replica quorum availability.
//! This models correlated failures; it does not promise consensus or durability.
use crate::distributed_membership::{Member, validate_membership};
use crate::distributed_weighted_quorum::WeightedQuorumPolicy;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneFaultAnalysis {
    pub read_failure_zones: Vec<String>,
    pub write_failure_zones: Vec<String>,
}

pub fn analyze_zone_faults(
    members: &[Member],
    policy: WeightedQuorumPolicy,
) -> Result<ZoneFaultAnalysis, &'static str> {
    validate_membership(members)?;
    WeightedQuorumPolicy::new(members, policy.reads, policy.writes)?;
    let mut weights = BTreeMap::<&str, u64>::new();
    for member in members {
        *weights.entry(member.zone.as_str()).or_insert(0) += u64::from(member.weight);
    }
    if weights.len() > 16 {
        return Err("zone fault enumeration exceeds 16-zone limit");
    }
    let zones: Vec<(&str, u64)> = weights.into_iter().collect();
    let total: u64 = zones.iter().map(|&(_, weight)| weight).sum();
    let mut best_read: Option<u32> = None;
    let mut best_write: Option<u32> = None;
    for mask in 1u32..(1u32 << zones.len()) {
        let lost: u64 = zones.iter().enumerate()
            .filter(|(i, _)| mask & (1u32 << i) != 0)
            .map(|(_, &(_, weight))| weight).sum();
        let remaining = total - lost;
        if remaining < policy.reads && best_read.is_none_or(|best| {
            (mask.count_ones(), mask) < (best.count_ones(), best)
        }) {
            best_read = Some(mask);
        }
        if remaining < policy.writes && best_write.is_none_or(|best| {
            (mask.count_ones(), mask) < (best.count_ones(), best)
        }) {
            best_write = Some(mask);
        }
    }
    let to_names = |mask: u32| {
        zones.iter().enumerate().filter_map(|(i, &(name, _))| {
            (mask & (1u32 << i) != 0).then(|| name.to_owned())
        }).collect::<Vec<_>>()
    };
    Ok(ZoneFaultAnalysis {
        read_failure_zones: to_names(best_read.ok_or("cannot find read quorum outage")?),
        write_failure_zones: to_names(best_write.ok_or("cannot find write quorum outage")?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(node: &str, zone: &str, weight: u16) -> Member {
        Member { node: node.into(), zone: zone.into(), weight }
    }

    #[test]
    fn two_zone_losses_break_three_zone_majority() {
        let m = [member("a", "east", 1), member("b", "west", 1), member("c", "central", 1)];
        let policy = WeightedQuorumPolicy::new(&m, 2, 2).unwrap();
        let faults = analyze_zone_faults(&m, policy).unwrap();
        assert_eq!(faults.read_failure_zones.len(), 2);
        assert_eq!(faults.write_failure_zones.len(), 2);
    }

    #[test]
    fn one_heavy_zone_can_disable_write_quorum() {
        let m = [member("a", "east", 4), member("b", "west", 1), member("c", "central", 1)];
        let policy = WeightedQuorumPolicy::new(&m, 2, 4).unwrap();
        let faults = analyze_zone_faults(&m, policy).unwrap();
        assert_eq!(faults.write_failure_zones, ["east"]);
        assert_eq!(faults.read_failure_zones.len(), 2);
    }

    #[test]
    fn rejects_unsafe_thresholds() {
        let m = [member("a", "z", 1)];
        assert!(analyze_zone_faults(&m, WeightedQuorumPolicy { reads: 0, writes: 1 }).is_err());
    }
}
