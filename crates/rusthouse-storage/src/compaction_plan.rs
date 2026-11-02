//! Safe, bounded size-tiered planning for immutable wide-part compaction.
//! No files are deleted or published by this module.

use std::collections::HashSet;

const MAX_PARTS: usize = 16_384;
const MAX_PLANS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartRange {
    pub name: String,
    pub partition: String,
    pub first_block: u64,
    pub last_block: u64,
    pub bytes: u64,
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionPair {
    pub left: String,
    pub right: String,
    pub partition: String,
    pub first_block: u64,
    pub last_block: u64,
    pub read_bytes: u64,
    pub resulting_level: u8,
}

fn identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Choose disjoint adjacent active-part pairs under a total read-byte budget.
///
/// Candidates are ordered by ascending read cost and deterministic partition,
/// block and name tie breakers. The algorithm is deliberately greedy, not a
/// guarantee of globally minimal compaction cost. Parts in different
/// partitions, at different merge levels, or with overlapping ranges
/// can never form a pair.
pub fn plan_compaction_pairs(
    parts: &[PartRange],
    max_pair_bytes: u64,
    byte_budget: u64,
    max_pairs: usize,
) -> Result<Vec<CompactionPair>, &'static str> {
    plan_compaction_pairs_with_ratio(parts, max_pair_bytes, byte_budget, max_pairs, None)
}

/// Refuse compaction pairs whose sizes differ by more than max_size_ratio.
///
/// The ratio is checked without overflowing even for u64::MAX part sizes.
/// The original planner remains available for callers that allow skew.
pub fn plan_balanced_compaction_pairs(
    parts: &[PartRange],
    max_pair_bytes: u64,
    byte_budget: u64,
    max_pairs: usize,
    max_size_ratio: u64,
) -> Result<Vec<CompactionPair>, &'static str> {
    if max_size_ratio == 0 {
        return Err("invalid compaction size ratio");
    }
    plan_compaction_pairs_with_ratio(
        parts, max_pair_bytes, byte_budget, max_pairs, Some(max_size_ratio),
    )
}

fn plan_compaction_pairs_with_ratio(
    parts: &[PartRange],
    max_pair_bytes: u64,
    byte_budget: u64,
    max_pairs: usize,
    max_size_ratio: Option<u64>,
) -> Result<Vec<CompactionPair>, &'static str> {
    if parts.len() > MAX_PARTS
        || max_pairs > MAX_PLANS
        || max_pair_bytes == 0
    {
        return Err("invalid compaction planning limits");
    }
    let mut seen = HashSet::with_capacity(parts.len());
    for part in parts {
        if !identifier(&part.name, 128)
            || !identifier(&part.partition, 128)
            || part.first_block > part.last_block
            || part.bytes == 0
            || part.level == u8::MAX
            || !seen.insert(part.name.as_str())
        {
            return Err("invalid or duplicate active part");
        }
    }
    let mut ordered = (0..parts.len()).collect::<Vec<_>>();
    ordered.sort_unstable_by(|&a, &b| {
        parts[a]
            .partition
            .cmp(&parts[b].partition)
            .then_with(|| parts[a].first_block.cmp(&parts[b].first_block))
            .then_with(|| parts[a].name.cmp(&parts[b].name))
    });
    for pair in ordered.windows(2) {
        let left = &parts[pair[0]];
        let right = &parts[pair[1]];
        if left.partition == right.partition && left.last_block >= right.first_block {
            return Err("active part block ranges overlap");
        }
    }

    let mut candidates = Vec::new();
    for pair in ordered.windows(2) {
        let left = &parts[pair[0]];
        let right = &parts[pair[1]];
        if left.partition != right.partition
            || left.level != right.level
            || left.last_block.checked_add(1) != Some(right.first_block)
        {
            continue;
        }
        let smaller = left.bytes.min(right.bytes);
        let larger = left.bytes.max(right.bytes);
        if max_size_ratio.is_some_and(|ratio| larger > smaller.saturating_mul(ratio)) {
            continue;
        }
        let cost = left
            .bytes
            .checked_add(right.bytes)
            .ok_or("compaction pair byte sum overflow")?;
        if cost <= max_pair_bytes {
            candidates.push((cost, pair[0], pair[1]));
        }
    }
    candidates.sort_unstable_by(|&(cost_a, a1, a2), &(cost_b, b1, b2)| {
        cost_a
            .cmp(&cost_b)
            .then_with(|| parts[a1].partition.cmp(&parts[b1].partition))
            .then_with(|| parts[a1].first_block.cmp(&parts[b1].first_block))
            .then_with(|| parts[a1].name.cmp(&parts[b1].name))
            .then_with(|| parts[a2].name.cmp(&parts[b2].name))
    });
    let mut used = vec![false; parts.len()];
    let mut remaining = byte_budget;
    let mut planned = Vec::new();
    for (cost, a, b) in candidates {
        if planned.len() == max_pairs {
            break;
        }
        if used[a] || used[b] || cost > remaining {
            continue;
        }
        used[a] = true;
        used[b] = true;
        remaining -= cost;
        let left = &parts[a];
        let right = &parts[b];
        planned.push(CompactionPair {
            left: left.name.clone(),
            right: right.name.clone(),
            partition: left.partition.clone(),
            first_block: left.first_block,
            last_block: right.last_block,
            read_bytes: cost,
            resulting_level: left.level + 1,
        });
    }
    planned.sort_unstable_by(|a, b| {
        a.partition
            .cmp(&b.partition)
            .then_with(|| a.first_block.cmp(&b.first_block))
    });
    Ok(planned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(name: &str, partition: &str, start: u64, bytes: u64) -> PartRange {
        PartRange {
            name: name.into(),
            partition: partition.into(),
            first_block: start,
            last_block: start,
            bytes,
            level: 0,
        }
    }

    #[test]
    fn chooses_cheapest_disjoint_pairs_within_budget() {
        let parts = vec![
            part("a", "p", 1, 100),
            part("b", "p", 2, 200),
            part("c", "p", 3, 10),
            part("d", "p", 4, 20),
            part("e", "p", 5, 30),
        ];
        let plans = plan_compaction_pairs(&parts, 500, 250, 2).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!((&plans[0].left[..], &plans[0].right[..]), ("c", "d"));
        assert_eq!(plans[0].read_bytes, 30);
        let plans = plan_compaction_pairs(&parts, 500, 600, 2).unwrap();
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].left, "a");
        assert_eq!(plans[1].left, "c");
        assert_eq!(plans[1].resulting_level, 1);
    }

    #[test]
    fn validates_input_without_relying_on_caller_order() {
        let parts = [part("b", "p", 2, 3), part("a", "p", 1, 4)];
        let plans = plan_compaction_pairs(&parts, 10, 10, 1).unwrap();
        assert_eq!(plans[0].left, "a");
        assert_eq!(plans[0].right, "b");
        let mut overlapping = parts.to_vec();
        overlapping[0].first_block = 1;
        assert!(plan_compaction_pairs(&overlapping, 10, 10, 1).is_err());
        let mut duplicated = parts.to_vec();
        duplicated[0].name = "a".into();
        assert!(plan_compaction_pairs(&duplicated, 10, 10, 1).is_err());
    }

    #[test]
    fn never_combines_partitions_or_levels() {
        let mut parts = vec![part("a", "p", 1, 1), part("b", "q", 2, 1)];
        assert!(plan_compaction_pairs(&parts, 10, 10, 1).unwrap().is_empty());
        parts[1].partition = "p".into();
        parts[1].level = 1;
        assert!(plan_compaction_pairs(&parts, 10, 10, 1).unwrap().is_empty());
    }

    #[test]
    fn does_not_span_missing_block_ranges_or_overflow_block_ids() {
        let parts = [
            part("a", "p", 1, 1),
            part("b", "p", 3, 1),
            part("c", "p", 4, 1),
        ];
        let planned = plan_compaction_pairs(&parts, 20, 20, 2).unwrap();
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].left, "b");
        assert_eq!(planned[0].right, "c");

        let high = [
            part("high1", "p", u64::MAX - 1, 1),
            part("high2", "p", u64::MAX, 1),
        ];
        let planned = plan_compaction_pairs(&high, 20, 20, 1).unwrap();
        assert_eq!(planned[0].last_block, u64::MAX);
    }

    #[test]
    fn balanced_planner_skips_skewed_pairs_but_retains_normal_pairs() {
        let unequal = [part("a", "p", 1, 1000), part("b", "p", 2, 1)];
        assert_eq!(plan_compaction_pairs(&unequal, 2000, 2000, 1).unwrap().len(), 1);
        assert!(plan_balanced_compaction_pairs(&unequal, 2000, 2000, 1, 4)
            .unwrap().is_empty());
        assert!(plan_balanced_compaction_pairs(&unequal, 2000, 2000, 1, 0).is_err());

        let parts = [
            part("a", "p", 1, 10),
            part("b", "p", 2, 30),
            part("c", "p", 3, 30),
            part("d", "p", 4, 10),
        ];
        let balanced = plan_balanced_compaction_pairs(&parts, 100, 100, 2, 2).unwrap();
        assert_eq!(balanced.len(), 1);
        assert_eq!((&balanced[0].left[..], &balanced[0].right[..]), ("b", "c"));

        let limits = [
            part("high", "p", 1, u64::MAX - 1),
            part("low", "p", 2, 1),
        ];
        assert!(plan_balanced_compaction_pairs(&limits, u64::MAX, u64::MAX, 1, u64::MAX).is_err());
    }

    #[test]
    fn overflow_and_zero_budgets_fail_closed() {
        let parts = [part("a", "p", 1, u64::MAX), part("b", "p", 2, 1)];
        assert!(plan_compaction_pairs(&parts, u64::MAX, u64::MAX, 1).is_err());
        let parts = [part("a", "p", 1, 1), part("b", "p", 2, 1)];
        assert!(plan_compaction_pairs(&parts, 2, 0, 1).unwrap().is_empty());
        assert!(plan_compaction_pairs(&parts, 0, 10, 1).is_err());
    }
}
