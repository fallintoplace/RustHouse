//! Strict, local replica fencing-token ledger.
//! Fencing tokens must be issued by an external authoritative coordinator;
//! these functions do not provide distributed locks or consensus.

use std::collections::{BTreeMap, HashSet};

const MAX_FENCED_SHARDS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FenceToken {
    pub epoch: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceClaim {
    pub shard: String,
    pub holder: String,
    pub token: FenceToken,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FencingLedger {
    claims: BTreeMap<String, (String, FenceToken)>,
}

fn identifier(name: &str, limit: usize) -> bool {
    !name.is_empty()
        && name.len() <= limit
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

impl FencingLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a set of token claims atomically in this local in-memory ledger.
    ///
    /// A retried identical token and holder is idempotent. Older tokens are
    /// rejected. A token cannot be transferred to another holder without
    /// advancing the externally issued sequence or epoch.
    pub fn apply_batch(&mut self, batch: &[FenceClaim]) -> Result<(), &'static str> {
        if batch.len() > MAX_FENCED_SHARDS {
            return Err("fencing claim batch exceeds limit");
        }
        let mut seen = HashSet::with_capacity(batch.len());
        let mut new_shards = 0usize;
        for claim in batch {
            if !identifier(&claim.shard, 128)
                || !identifier(&claim.holder, 64)
                || claim.token.epoch == 0
                || claim.token.sequence == 0
                || !seen.insert(claim.shard.as_str())
            {
                return Err("invalid or duplicate fencing claim");
            }
            match self.claims.get(&claim.shard) {
                Some((holder, token)) => {
                    if claim.token < *token
                        || (claim.token == *token && claim.holder != *holder)
                    {
                        return Err("stale or conflicting fencing token");
                    }
                }
                None => new_shards += 1,
            }
        }
        if self.claims.len() + new_shards > MAX_FENCED_SHARDS {
            return Err("fencing ledger exceeds shard limit");
        }
        // All validation is complete before the first write. No request from
        // the failed batch can change the ledger.
        for claim in batch {
            self.claims.insert(
                claim.shard.clone(),
                (claim.holder.clone(), claim.token),
            );
        }
        Ok(())
    }

    /// Apply a batch only when every shard still has its expected token.
    ///
    /// Expectations are positional and correspond to batch claims. None means
    /// that the shard must not exist yet. All checks and updates are local
    /// to this ledger; external coordination still issues fencing tokens.
    pub fn apply_batch_if_tokens_match(
        &mut self,
        batch: &[FenceClaim],
        expected: &[Option<FenceToken>],
    ) -> Result<(), &'static str> {
        if batch.len() != expected.len() || batch.len() > MAX_FENCED_SHARDS {
            return Err("invalid conditional fencing batch");
        }
        for (claim, &expected_token) in batch.iter().zip(expected) {
            let current_token = self.claims.get(&claim.shard).map(|(_, token)| *token);
            if current_token != expected_token {
                return Err("fencing claim precondition failed");
            }
        }
        self.apply_batch(batch)
    }

    /// Return the current local holder and token for a known shard.
    pub fn get(&self, shard: &str) -> Option<(&str, FenceToken)> {
        self.claims
            .get(shard)
            .map(|(holder, token)| (holder.as_str(), *token))
    }

    pub fn len(&self) -> usize {
        self.claims.len()
    }

    pub fn is_empty(&self) -> bool {
        self.claims.is_empty()
    }

    /// Canonical deterministic order for explicit, externally durable state.
    pub fn snapshot(&self) -> Vec<FenceClaim> {
        self.claims
            .iter()
            .map(|(shard, (holder, token))| FenceClaim {
                shard: shard.clone(),
                holder: holder.clone(),
                token: *token,
            })
            .collect()
    }

    /// Validate an externally persisted complete ledger before using it.
    pub fn restore(records: &[FenceClaim]) -> Result<Self, &'static str> {
        let mut ledger = Self::new();
        ledger.apply_batch(records)?;
        Ok(ledger)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(shard: &str, holder: &str, epoch: u64, sequence: u64) -> FenceClaim {
        FenceClaim {
            shard: shard.into(),
            holder: holder.into(),
            token: FenceToken { epoch, sequence },
        }
    }

    #[test]
    fn only_monotonic_claims_can_change_owner() {
        let mut ledger = FencingLedger::new();
        ledger.apply_batch(&[claim("alpha", "node1", 1, 7)]).unwrap();
        ledger.apply_batch(&[claim("alpha", "node1", 1, 7)]).unwrap();
        assert!(ledger.apply_batch(&[claim("alpha", "node2", 1, 7)]).is_err());
        assert!(ledger.apply_batch(&[claim("alpha", "node2", 1, 6)]).is_err());
        ledger.apply_batch(&[claim("alpha", "node2", 1, 8)]).unwrap();
        assert!(ledger.apply_batch(&[claim("alpha", "node1", 1, 7)]).is_err());
        ledger.apply_batch(&[claim("alpha", "node3", 2, 1)]).unwrap();
        assert_eq!(ledger.get("alpha").unwrap().0, "node3");
        assert!(ledger.apply_batch(&[claim("alpha", "node4", 1, u64::MAX)]).is_err());
    }

    #[test]
    fn a_bad_claim_rolls_back_every_change_in_batch() {
        let mut ledger = FencingLedger::new();
        ledger.apply_batch(&[claim("a", "n", 1, 1)]).unwrap();
        let before = ledger.clone();
        assert!(ledger.apply_batch(&[
            claim("a", "new", 2, 1),
            claim("b", "n", 1, 1),
            claim("a", "again", 3, 1),
        ]).is_err());
        assert_eq!(ledger, before);
        assert!(ledger.apply_batch(&[
            claim("b", "n", 1, 1),
            claim("a", "new", 1, 1),
        ]).is_err());
        assert_eq!(ledger, before);
    }

    #[test]
    fn snapshot_and_restore_are_canonical() {
        let mut ledger = FencingLedger::new();
        ledger.apply_batch(&[
            claim("z", "replica", 4, 20),
            claim("a", "replica", 1, 1),
        ]).unwrap();
        let snapshot = ledger.snapshot();
        assert_eq!(snapshot[0].shard, "a");
        assert_eq!(snapshot[1].shard, "z");
        assert_eq!(FencingLedger::restore(&snapshot).unwrap(), ledger);
    }

    #[test]
    fn conditional_batches_fail_atomically_on_stale_snapshot() {
        let mut ledger = FencingLedger::new();
        ledger.apply_batch(&[claim("alpha", "old", 1, 1)]).unwrap();
        let initial = FenceToken { epoch: 1, sequence: 1 };
        let batch = [claim("alpha", "new", 1, 2), claim("beta", "new", 1, 1)];

        let before = ledger.clone();
        assert!(ledger.apply_batch_if_tokens_match(&batch, &[None, None]).is_err());
        assert_eq!(ledger, before);
        assert!(ledger.apply_batch_if_tokens_match(&batch, &[Some(initial)]).is_err());
        assert_eq!(ledger, before);

        ledger
            .apply_batch_if_tokens_match(&batch, &[Some(initial), None])
            .unwrap();
        assert_eq!(ledger.get("alpha").unwrap().0, "new");
        assert_eq!(ledger.get("beta").unwrap().0, "new");
        let after = ledger.clone();

        assert!(ledger
            .apply_batch_if_tokens_match(&[claim("alpha", "old", 2, 1)], &[Some(initial)])
            .is_err());
        assert_eq!(ledger, after);
        assert!(ledger.apply_batch_if_tokens_match(&batch, &[Some(initial), None]).is_err());
        assert_eq!(ledger, after);
    }

    #[test]
    fn validates_identifiers_and_zero_tokens() {
        let mut ledger = FencingLedger::new();
        assert!(ledger.apply_batch(&[claim("x/y", "node", 1, 1)]).is_err());
        assert!(ledger.apply_batch(&[claim("x", "", 1, 1)]).is_err());
        assert!(ledger.apply_batch(&[claim("x", "node", 0, 1)]).is_err());
        assert!(ledger.apply_batch(&[claim("x", "node", 1, 0)]).is_err());
        assert!(ledger.is_empty());
    }
}


/// Optimistic, externally authorized replacement of a local fencing claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedFenceUpdate {
    /// The exact previously observed claim, or None for an absent shard.
    pub expected: Option<FenceClaim>,
    pub next: FenceClaim,
}

impl FencingLedger {
    /// Compare all prior claims before installing any new fencing tokens.
    ///
    /// This rejects stale plans even when they carry a newer token than the
    /// ledger. As with apply_batch, the caller must obtain new tokens from an
    /// authoritative coordinator and durably persist the resulting state.
    pub fn apply_checked_batch(
        &mut self,
        updates: &[CheckedFenceUpdate],
    ) -> Result<(), &'static str> {
        if updates.len() > MAX_FENCED_SHARDS {
            return Err("checked fencing batch exceeds limit");
        }
        let mut seen = HashSet::with_capacity(updates.len());
        let mut claims = Vec::with_capacity(updates.len());
        for update in updates {
            if !seen.insert(update.next.shard.as_str()) {
                return Err("duplicate checked fencing shard");
            }
            let matches_current = match (
                self.get(&update.next.shard),
                update.expected.as_ref(),
            ) {
                (None, None) => true,
                (Some((holder, token)), Some(expected)) => {
                    expected.shard == update.next.shard
                        && expected.holder == holder
                        && expected.token == token
                }
                _ => false,
            };
            if !matches_current {
                return Err("stale fencing compare-and-swap expectation");
            }
            claims.push(update.next.clone());
        }
        // All expectations are checked first. apply_batch validates every new
        // claim and the capacity before changing the ledger.
        self.apply_batch(&claims)
    }
}

#[cfg(test)]
mod checked_fencing_tests {
    use super::*;

    fn claim(shard: &str, holder: &str, sequence: u64) -> FenceClaim {
        FenceClaim {
            shard: shard.into(),
            holder: holder.into(),
            token: FenceToken { epoch: 1, sequence },
        }
    }

    #[test]
    fn compare_and_swap_rejects_stale_plans_with_newer_tokens() {
        let mut ledger = FencingLedger::new();
        let original = claim("one", "a", 1);
        ledger.apply_batch(std::slice::from_ref(&original)).unwrap();
        let replacement = CheckedFenceUpdate {
            expected: Some(original.clone()),
            next: claim("one", "b", 2),
        };
        ledger
            .apply_checked_batch(std::slice::from_ref(&replacement))
            .unwrap();
        assert_eq!(ledger.get("one").unwrap().0, "b");

        let stale = CheckedFenceUpdate {
            expected: Some(original),
            next: claim("one", "c", 100),
        };
        let before = ledger.clone();
        assert!(ledger.apply_checked_batch(&[stale]).is_err());
        assert_eq!(ledger, before);
    }

    #[test]
    fn partial_failures_cannot_publish_any_checked_claims() {
        let mut ledger = FencingLedger::new();
        let first = claim("first", "owner", 2);
        ledger.apply_batch(std::slice::from_ref(&first)).unwrap();
        let before = ledger.clone();

        let good = CheckedFenceUpdate {
            expected: None,
            next: claim("new", "owner", 1),
        };
        let bad = CheckedFenceUpdate {
            expected: None,
            next: claim("first", "another", 99),
        };
        assert!(ledger.apply_checked_batch(&[good.clone(), bad]).is_err());
        assert_eq!(ledger, before);

        let duplicate = CheckedFenceUpdate {
            expected: None,
            next: claim("new", "owner", 2),
        };
        assert!(ledger.apply_checked_batch(&[good.clone(), duplicate]).is_err());
        assert_eq!(ledger, before);

        ledger.apply_checked_batch(&[good]).unwrap();
        assert_eq!(ledger.get("new").unwrap().1.sequence, 1);
    }
}
