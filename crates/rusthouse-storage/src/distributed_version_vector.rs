//! Version-vector causality for experimental replica metadata.
//! No vector comparison by itself establishes a committed write.
use std::collections::BTreeMap;

const MAX_NODES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CausalRelation {
    Equal,
    Before,
    After,
    Concurrent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionVector {
    counters: BTreeMap<String, u64>,
}

fn valid_node(node: &str) -> bool {
    !node.is_empty() && node.len() <= 64
        && node.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl VersionVector {
    pub fn from_counters(entries: &[(&str, u64)]) -> Result<Self, &'static str> {
        if entries.len() > MAX_NODES { return Err("too many version-vector members"); }
        let mut counters = BTreeMap::new();
        for &(node, count) in entries {
            if !valid_node(node) || count == 0
                || counters.insert(node.to_owned(), count).is_some()
            {
                return Err("invalid or duplicate version-vector entry");
            }
        }
        Ok(Self { counters })
    }

    pub fn counter(&self, node: &str) -> u64 {
        self.counters.get(node).copied().unwrap_or(0)
    }

    pub fn increment(&mut self, node: &str) -> Result<u64, &'static str> {
        if !valid_node(node) {
            return Err("invalid version-vector member");
        }
        if !self.counters.contains_key(node) && self.counters.len() == MAX_NODES {
            return Err("version-vector membership limit reached");
        }
        let next = self.counter(node).checked_add(1)
            .ok_or("version-vector counter overflow")?;
        self.counters.insert(node.to_owned(), next);
        Ok(next)
    }

    /// Merge by component-wise maximum. The receiver is unchanged on error.
    pub fn merge(&mut self, other: &Self) -> Result<(), &'static str> {
        if self.counters.len() + other.counters.keys()
            .filter(|node| !self.counters.contains_key(*node)).count() > MAX_NODES
        {
            return Err("merged version-vector membership limit reached");
        }
        for (node, &count) in &other.counters {
            let entry = self.counters.entry(node.clone()).or_insert(0);
            *entry = (*entry).max(count);
        }
        Ok(())
    }

    pub fn relation(&self, other: &Self) -> CausalRelation {
        let mut smaller = false;
        let mut greater = false;
        for (node, &count) in &self.counters {
            let remote = other.counter(node);
            smaller |= count < remote;
            greater |= count > remote;
        }
        for (node, &count) in &other.counters {
            if !self.counters.contains_key(node) && count > 0 {
                smaller = true;
            }
        }
        match (smaller, greater) {
            (false, false) => CausalRelation::Equal,
            (true, false) => CausalRelation::Before,
            (false, true) => CausalRelation::After,
            (true, true) => CausalRelation::Concurrent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_updates_are_concurrent_until_merged() {
        let mut a = VersionVector::default();
        let mut b = VersionVector::default();
        a.increment("a").unwrap();
        b.increment("b").unwrap();
        assert_eq!(a.relation(&b), CausalRelation::Concurrent);
        a.merge(&b).unwrap();
        assert_eq!(a.relation(&b), CausalRelation::After);
        b.merge(&a).unwrap();
        assert_eq!(a.relation(&b), CausalRelation::Equal);
    }

    #[test]
    fn rejects_duplicate_and_overflow_counters_without_changes() {
        assert!(VersionVector::from_counters(&[("a", 1), ("a", 2)]).is_err());
        let mut vv = VersionVector::from_counters(&[("a", u64::MAX)]).unwrap();
        let old = vv.clone();
        assert!(vv.increment("a").is_err());
        assert_eq!(vv, old);
    }

    #[test]
    fn strict_causal_dominance_includes_missing_counters() {
        let a = VersionVector::from_counters(&[("a", 2), ("b", 1)]).unwrap();
        let b = VersionVector::from_counters(&[("a", 1)]).unwrap();
        assert_eq!(b.relation(&a), CausalRelation::Before);
        assert_eq!(a.relation(&b), CausalRelation::After);
    }
}
