//! Exact budgeted repair selection via meet-in-the-middle subset enumeration.
//! This schedules metadata only, not I/O or network transfers.
use std::collections::HashSet;
const MAX_JOBS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepairCandidate {
    pub id: String,
    pub bytes: u64,
    pub priority: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepairSelection {
    pub ids: Vec<String>,
    pub bytes: u64,
    pub priority: u64,
}

#[derive(Clone, Copy)]
struct Choice { mask: u64, bytes: u64, priority: u64 }

fn better(a: Choice, b: Choice) -> bool {
    (a.priority, std::cmp::Reverse(a.bytes), std::cmp::Reverse(a.mask))
        > (b.priority, std::cmp::Reverse(b.bytes), std::cmp::Reverse(b.mask))
}

fn choices(items: &[RepairCandidate], budget: u64) -> Vec<Choice> {
    let mut out = Vec::new();
    let mut bytes = 0u128;
    let mut priority = 0u64;
    let mut previous = 0usize;
    // Gray-code enumeration changes one candidate between adjacent subsets.
    // Track exact u128 bytes so large infeasible subsets never overflow.
    for ordinal in 0..(1usize << items.len()) {
        let mask = ordinal ^ (ordinal >> 1);
        let changed = mask ^ previous;
        if changed != 0 {
            let index = changed.trailing_zeros() as usize;
            let item = &items[index];
            if mask & changed != 0 {
                bytes += u128::from(item.bytes);
                priority += u64::from(item.priority);
            } else {
                bytes -= u128::from(item.bytes);
                priority -= u64::from(item.priority);
            }
        }
        if bytes <= u128::from(budget) {
            out.push(Choice { mask: mask as u64, bytes: bytes as u64, priority });
        }
        previous = mask;
    }
    out
}

pub fn select_repair_batch(
    candidates: &[RepairCandidate],
    budget: u64,
) -> Result<RepairSelection, &'static str> {
    if candidates.len() > MAX_JOBS {
        return Err("too many repair knapsack candidates");
    }
    let mut seen = HashSet::new();
    for candidate in candidates {
        if candidate.id.is_empty() || candidate.id.len() > 64
            || !candidate.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || candidate.bytes == 0 || !seen.insert(candidate.id.as_str())
        {
            return Err("invalid repair knapsack candidate");
        }
    }
    let mid = candidates.len() / 2;
    let mut left = choices(&candidates[..mid], budget);
    left.sort_unstable_by_key(|c| (c.bytes, std::cmp::Reverse(c.priority), c.mask));
    let mut prefix = Vec::with_capacity(left.len());
    let mut best_left = Choice { mask: 0, bytes: 0, priority: 0 };
    for choice in &left {
        if better(*choice, best_left) { best_left = *choice; }
        prefix.push(best_left);
    }
    let mut best = Choice { mask: 0, bytes: 0, priority: 0 };
    for right in choices(&candidates[mid..], budget) {
        let remaining = budget - right.bytes;
        let count = left.partition_point(|candidate| candidate.bytes <= remaining);
        if count == 0 { continue; }
        let paired = prefix[count - 1];
        let combined = Choice {
            mask: paired.mask | (right.mask << mid),
            bytes: paired.bytes + right.bytes,
            priority: paired.priority + right.priority,
        };
        if better(combined, best) { best = combined; }
    }
    let ids = candidates.iter().enumerate().filter_map(|(i, c)| {
        (best.mask & (1u64 << i) != 0).then(|| c.id.clone())
    }).collect();
    Ok(RepairSelection { ids, bytes: best.bytes, priority: best.priority })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: &str, bytes: u64, priority: u32) -> RepairCandidate {
        RepairCandidate { id: id.into(), bytes, priority }
    }
    #[test]
    fn exact_solution_beats_greedy() {
        let items = [item("large", 6, 10), item("small1", 5, 9), item("small2", 5, 9)];
        let plan = select_repair_batch(&items, 10).unwrap();
        assert_eq!(plan.ids, ["small1", "small2"]);
        assert_eq!((plan.bytes, plan.priority), (10, 18));
    }
    #[test]
    fn zero_budget_and_ties_are_deterministic() {
        let items = [item("a", 1, 5), item("b", 1, 5)];
        assert!(select_repair_batch(&items, 0).unwrap().ids.is_empty());
        assert_eq!(select_repair_batch(&items, 1).unwrap().ids, ["a"]);
    }
    #[test]
    fn rejects_duplicate_or_zero_byte_jobs() {
        assert!(select_repair_batch(&[item("a", 1, 1), item("a", 2, 2)], 5).is_err());
        assert!(select_repair_batch(&[item("a", 0, 1)], 5).is_err());
    }
}

#[cfg(test)]
mod gray_code_enumeration_tests {
    use super::*;

    #[test]
    fn subset_selection_matches_independent_exhaustive_oracle() {
        let jobs = (0..12)
            .map(|index| RepairCandidate {
                id: format!("shard_{index}"),
                bytes: if index == 0 { u64::MAX - 2 } else { (index * 7 % 19 + 1) as u64 },
                priority: (index * 13 % 11 + 1) as u32,
            })
            .collect::<Vec<_>>();
        for budget in [0, 1, 20, 50, 100, u64::MAX] {
            let mut expected = Choice { mask: 0, bytes: 0, priority: 0 };
            for mask in 0..(1usize << jobs.len()) {
                let mut bytes = 0u128;
                let mut priority = 0u64;
                for (index, job) in jobs.iter().enumerate() {
                    if mask & (1 << index) != 0 {
                        bytes += u128::from(job.bytes);
                        priority += u64::from(job.priority);
                    }
                }
                if bytes <= u128::from(budget) {
                    let candidate = Choice { mask: mask as u64, bytes: bytes as u64, priority };
                    if better(candidate, expected) {
                        expected = candidate;
                    }
                }
            }
            let expected_ids = jobs.iter().enumerate()
                .filter_map(|(index, job)| {
                    (expected.mask & (1 << index) != 0).then(|| job.id.clone())
                })
                .collect::<Vec<_>>();
            let selected = select_repair_batch(&jobs, budget).unwrap();
            assert_eq!(selected.ids, expected_ids, "budget={budget}");
            assert_eq!((selected.bytes, selected.priority), (expected.bytes, expected.priority));
        }
    }
}
