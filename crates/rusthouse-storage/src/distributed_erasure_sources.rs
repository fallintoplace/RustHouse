//! Exact source selection for an erasure-coded recovery operation.
//! Only chooses transfer metadata, not data decoding or repair writes.
use std::collections::HashSet;

const MAX_SOURCES: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentSource {
    pub node: String,
    pub zone: String,
    pub fragment: u8,
    pub transfer_cost: u64,
    pub healthy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErasureSourcePlan {
    pub nodes: Vec<String>,
    pub fragment_indices: Vec<u8>,
    pub total_cost: u64,
}

/// Pick exactly required distinct fragments from separate source nodes.
/// At least min_zones different zones must be represented.
pub fn choose_erasure_sources(
    sources: &[FragmentSource],
    required: usize,
    min_zones: usize,
) -> Result<ErasureSourcePlan, &'static str> {
    if sources.is_empty() || sources.len() > MAX_SOURCES
        || required == 0 || required > 16 || required > sources.len()
        || min_zones == 0 || min_zones > required
    {
        return Err("invalid erasure recovery selection limits");
    }
    let mut nodes = HashSet::new();
    for source in sources {
        if source.node.is_empty() || source.node.len() > 64
            || source.zone.is_empty() || source.zone.len() > 64
            || !source.node.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !source.zone.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || source.fragment >= 128 || source.transfer_cost == 0
            || !nodes.insert(source.node.as_str())
        {
            return Err("invalid erasure recovery source");
        }
    }
    let mut chosen_mask = None;
    let mut chosen_cost = u64::MAX;
    for mask in 1u32..(1u32 << sources.len()) {
        if mask.count_ones() as usize != required { continue; }
        let mut fragments = 0u128;
        let mut zones = HashSet::new();
        let mut cost = 0u64;
        let mut good = true;
        for (index, source) in sources.iter().enumerate() {
            if mask & (1u32 << index) == 0 { continue; }
            let bit = 1u128 << source.fragment;
            if !source.healthy || fragments & bit != 0 {
                good = false;
                break;
            }
            fragments |= bit;
            zones.insert(source.zone.as_str());
            let Some(next) = cost.checked_add(source.transfer_cost) else {
                good = false;
                break;
            };
            cost = next;
        }
        if good && zones.len() >= min_zones
            && (chosen_mask.is_none() || (cost, mask) < (chosen_cost, chosen_mask.unwrap()))
        {
            chosen_cost = cost;
            chosen_mask = Some(mask);
        }
    }
    let mask = chosen_mask.ok_or("no feasible erasure fragment source set")?;
    let mut nodes = Vec::new();
    let mut fragment_indices = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        if mask & (1u32 << index) != 0 {
            nodes.push(source.node.clone());
            fragment_indices.push(source.fragment);
        }
    }
    Ok(ErasureSourcePlan { nodes, fragment_indices, total_cost: chosen_cost })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(node: &str, zone: &str, fragment: u8, cost: u64) -> FragmentSource {
        FragmentSource { node: node.into(), zone: zone.into(), fragment,
            transfer_cost: cost, healthy: true }
    }

    #[test]
    fn exact_source_plan_does_not_download_duplicate_fragments() {
        let candidates = [
            source("a", "east", 0, 1),
            source("b", "east", 0, 1),
            source("c", "west", 1, 3),
            source("d", "west", 2, 10),
        ];
        let plan = choose_erasure_sources(&candidates, 2, 2).unwrap();
        assert_eq!(plan.total_cost, 4);
        assert_eq!(plan.nodes, ["a", "c"]);
        assert_eq!(plan.fragment_indices, [0, 1]);
    }

    #[test]
    fn missing_zone_diversity_fails_closed() {
        let items = [source("a", "east", 0, 1), source("b", "east", 1, 2)];
        assert!(choose_erasure_sources(&items, 2, 2).is_err());
    }

    #[test]
    fn overflow_and_unhealthy_fragment_cannot_form_a_plan() {
        let mut items = [source("a", "east", 0, u64::MAX),
            source("b", "west", 1, 2)];
        assert!(choose_erasure_sources(&items, 2, 2).is_err());
        items[0].healthy = false;
        assert!(choose_erasure_sources(&items, 2, 2).is_err());
    }
}
