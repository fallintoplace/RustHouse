//! Exact source selection for an erasure-coded recovery operation.
//! Only chooses transfer metadata, not data decoding or repair writes.
use std::collections::HashSet;

const MAX_SOURCES: usize = 20;

fn valid_source_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

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

fn validate_source_request(
    sources: &[FragmentSource],
    required: usize,
    min_zones: usize,
) -> Result<(), &'static str> {
    if sources.is_empty()
        || sources.len() > MAX_SOURCES
        || required == 0
        || required > 16
        || required > sources.len()
        || min_zones == 0
        || min_zones > required
    {
        return Err("invalid erasure recovery selection limits");
    }
    let mut nodes = HashSet::new();
    for source in sources {
        if !valid_source_label(&source.node)
            || !valid_source_label(&source.zone)
            || source.fragment >= 128
            || source.transfer_cost == 0
            || !nodes.insert(source.node.as_str())
        {
            return Err("invalid erasure recovery source");
        }
    }
    Ok(())
}

fn selection_cost(ordered: &[&FragmentSource], mask: u32, min_zones: usize) -> Option<u64> {
    let mut fragments = 0u128;
    let mut zones = HashSet::new();
    let mut cost = 0u64;
    for (index, source) in ordered.iter().enumerate() {
        if mask & (1u32 << index) == 0 {
            continue;
        }
        let bit = 1u128 << source.fragment;
        if !source.healthy || fragments & bit != 0 {
            return None;
        }
        fragments |= bit;
        zones.insert(source.zone.as_str());
        cost = cost.checked_add(source.transfer_cost)?;
    }
    (zones.len() >= min_zones).then_some(cost)
}

/// Pick exactly required distinct fragments from separate source nodes.
/// At least min_zones different zones must be represented.
pub fn choose_erasure_sources(
    sources: &[FragmentSource],
    required: usize,
    min_zones: usize,
) -> Result<ErasureSourcePlan, &'static str> {
    validate_source_request(sources, required, min_zones)?;
    // Canonicalize source enumeration so equal-cost plans do not depend on
    // the order in which replica metadata arrived.
    let mut ordered = sources.iter().collect::<Vec<_>>();
    ordered.sort_unstable_by(|left, right| left.node.cmp(&right.node));

    let mut chosen = None;
    for mask in 1u32..(1u32 << sources.len()) {
        if mask.count_ones() as usize != required {
            continue;
        }
        if let Some(cost) = selection_cost(&ordered, mask, min_zones) {
            if chosen.is_none_or(|best| (cost, mask) < best) {
                chosen = Some((cost, mask));
            }
        }
    }
    let (chosen_cost, mask) = chosen.ok_or("no feasible erasure fragment source set")?;
    let mut nodes = Vec::new();
    let mut fragment_indices = Vec::new();
    for (index, source) in ordered.iter().enumerate() {
        if mask & (1u32 << index) != 0 {
            nodes.push(source.node.clone());
            fragment_indices.push(source.fragment);
        }
    }
    Ok(ErasureSourcePlan {
        nodes,
        fragment_indices,
        total_cost: chosen_cost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(node: &str, zone: &str, fragment: u8, cost: u64) -> FragmentSource {
        FragmentSource {
            node: node.into(),
            zone: zone.into(),
            fragment,
            transfer_cost: cost,
            healthy: true,
        }
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
        let mut items = [source("a", "east", 0, u64::MAX), source("b", "west", 1, 2)];
        assert!(choose_erasure_sources(&items, 2, 2).is_err());
        items[0].healthy = false;
        assert!(choose_erasure_sources(&items, 2, 2).is_err());
    }
}

#[cfg(test)]
mod stable_source_order_tests {
    use super::*;

    fn source(node: &str, fragment: u8) -> FragmentSource {
        FragmentSource {
            node: node.into(),
            zone: "zone".into(),
            fragment,
            transfer_cost: 1,
            healthy: true,
        }
    }

    #[test]
    fn tied_recovery_costs_choose_same_nodes_for_every_input_order() {
        let sources = [source("c", 2), source("a", 0), source("b", 1)];
        let plan = choose_erasure_sources(&sources, 2, 1).unwrap();
        assert_eq!(plan.total_cost, 2);
        assert_eq!(plan.nodes, ["a", "b"]);
        assert_eq!(plan.fragment_indices, [0, 1]);

        let shuffled = [sources[2].clone(), sources[0].clone(), sources[1].clone()];
        assert_eq!(choose_erasure_sources(&shuffled, 2, 1).unwrap(), plan);
    }
}
