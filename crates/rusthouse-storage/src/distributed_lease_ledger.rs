//! Time-bounded local lease ledger using externally issued fencing tokens.
//! Does not issue tokens, elect leaders, or implement distributed locks.
use crate::distributed_fencing::FenceToken;
use std::collections::{BTreeMap, HashSet};
const MAX_LEASES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedLease {
    pub resource: String,
    pub holder: String,
    pub token: FenceToken,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TimedLeaseLedger {
    last_now_ms: u64,
    entries: BTreeMap<String, TimedLease>,
}

fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl TimedLeaseLedger {
    pub fn new() -> Self { Self::default() }

    pub fn apply(
        &mut self,
        proposed: TimedLease,
        now_ms: u64,
        max_ttl_ms: u64,
    ) -> Result<(), &'static str> {
        if !identifier(&proposed.resource) || !identifier(&proposed.holder)
            || proposed.token.epoch == 0 || proposed.token.sequence == 0
            || now_ms < self.last_now_ms || max_ttl_ms == 0
            || proposed.expires_at_ms <= now_ms
            || proposed.expires_at_ms > now_ms.saturating_add(max_ttl_ms)
        {
            return Err("invalid replica lease, token, or clock");
        }
        if let Some(existing) = self.entries.get(&proposed.resource) {
            if proposed.token < existing.token
                || (proposed.token == existing.token
                    && (proposed.holder != existing.holder
                        || proposed.expires_at_ms < existing.expires_at_ms))
            {
                return Err("stale or conflicting replica lease");
            }
        } else if self.entries.len() == MAX_LEASES {
            return Err("replica lease ledger exceeds limit");
        }
        self.last_now_ms = now_ms;
        self.entries.insert(proposed.resource.clone(), proposed);
        Ok(())
    }


    /// Apply a set of externally fenced leases without partial local writes.
    pub fn apply_batch(
        &mut self,
        leases: &[TimedLease],
        now_ms: u64,
        max_ttl_ms: u64,
    ) -> Result<(), &'static str> {
        if leases.len() > MAX_LEASES {
            return Err("replica lease batch exceeds limit");
        }
        let mut seen = HashSet::new();
        if leases.iter().any(|lease| !seen.insert(lease.resource.as_str())) {
            return Err("duplicate resource in replica lease batch");
        }
        let mut staged = self.clone();
        for lease in leases {
            staged.apply(lease.clone(), now_ms, max_ttl_ms)?;
        }
        *self = staged;
        Ok(())
    }

    /// Expiry does not remove high-water tokens from the ledger.
    pub fn active(&self, resource: &str, now_ms: u64) -> Option<&TimedLease> {
        self.entries.get(resource).filter(|entry| entry.expires_at_ms > now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lease(holder: &str, seq: u64, expires: u64) -> TimedLease {
        TimedLease {
            resource: "shard".into(), holder: holder.into(),
            token: FenceToken { epoch: 1, sequence: seq }, expires_at_ms: expires,
        }
    }

    #[test]
    fn stale_holder_cannot_retake_after_expiry() {
        let mut ledger = TimedLeaseLedger::new();
        ledger.apply(lease("a", 5, 110), 100, 20).unwrap();
        assert!(ledger.active("shard", 111).is_none());
        assert!(ledger.apply(lease("b", 4, 130), 120, 20).is_err());
        ledger.apply(lease("b", 6, 130), 120, 20).unwrap();
        assert_eq!(ledger.active("shard", 125).unwrap().holder, "b");
    }

    #[test]
    fn clock_rollback_or_shortening_is_rejected_without_mutation() {
        let mut ledger = TimedLeaseLedger::new();
        ledger.apply(lease("a", 5, 110), 100, 20).unwrap();
        assert!(ledger.apply(lease("a", 5, 109), 100, 20).is_err());
        assert!(ledger.apply(lease("a", 5, 130), 99, 40).is_err());
        assert_eq!(ledger.active("shard", 109).unwrap().expires_at_ms, 110);
    }
}

#[cfg(test)]
mod lease_batch_tests {
    use super::*;

    fn lease(resource: &str, holder: &str, token: u64, expiry: u64) -> TimedLease {
        TimedLease {
            resource: resource.into(),
            holder: holder.into(),
            token: FenceToken { epoch: 1, sequence: token },
            expires_at_ms: expiry,
        }
    }

    #[test]
    fn invalid_lease_rolls_back_all_other_batch_entries() {
        let mut ledger = TimedLeaseLedger::new();
        ledger.apply(lease("shard", "a", 5, 140), 100, 50).unwrap();
        let before = ledger.clone();
        let new_lease = lease("other", "b", 1, 145);
        let bad = lease("shard", "b", 5, 145);
        assert!(ledger.apply_batch(&[new_lease.clone(), bad], 110, 50).is_err());
        assert_eq!(ledger.entries, before.entries);
        assert_eq!(ledger.last_now_ms, before.last_now_ms);

        ledger.apply_batch(&[new_lease, lease("shard", "b", 6, 145)], 110, 50).unwrap();
        assert_eq!(ledger.active("shard", 111).unwrap().holder, "b");
        assert!(ledger.active("other", 111).is_some());
    }

    #[test]
    fn repeated_resource_is_not_a_valid_batch() {
        let mut ledger = TimedLeaseLedger::new();
        assert!(ledger.apply_batch(&[
            lease("shard", "a", 1, 20), lease("shard", "b", 2, 20),
        ], 10, 20).is_err());
        assert!(ledger.entries.is_empty());
    }
}
