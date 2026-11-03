//! Bounded, in-memory idempotency ledger for externally fenced replica requests.
//! Production exactly-once effects require persisting this ledger atomically.
use std::collections::BTreeMap;
const MAX_REQUESTS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RequestKey {
    pub node: String,
    pub epoch: u64,
    pub nonce: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    New,
    Replayed,
}

#[derive(Debug, Clone)]
struct Entry {
    fingerprint: u128,
    expires_at_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct IdempotencyLedger {
    requests: BTreeMap<RequestKey, Entry>,
    highest_epoch: BTreeMap<String, u64>,
    last_time_ms: u64,
}

impl IdempotencyLedger {
    pub fn new() -> Self { Self::default() }


    /// Validate a batch without partially changing local replay or epoch state.
    /// Callers must still commit external effects and this ledger durably.
    pub fn admit_batch(
        &mut self,
        requests: &[(RequestKey, u128)],
        now_ms: u64,
        ttl_ms: u64,
    ) -> Result<Vec<Admission>, &'static str> {
        if requests.len() > MAX_REQUESTS {
            return Err("replica idempotency batch exceeds limit");
        }
        let mut staged = self.clone();
        let mut results = Vec::with_capacity(requests.len());
        for (key, fingerprint) in requests {
            results.push(staged.admit(key.clone(), *fingerprint, now_ms, ttl_ms)?);
        }
        *self = staged;
        Ok(results)
    }

    pub fn admit(
        &mut self,
        key: RequestKey,
        payload_fingerprint: u128,
        now_ms: u64,
        ttl_ms: u64,
    ) -> Result<Admission, &'static str> {
        if key.node.is_empty() || key.node.len() > 64
            || !key.node.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || key.epoch == 0 || key.nonce == 0 || ttl_ms == 0
            || now_ms < self.last_time_ms
        {
            return Err("invalid or regressing replica request");
        }
        let expiry = now_ms.checked_add(ttl_ms).ok_or("idempotency TTL overflow")?;
        if self.highest_epoch.get(&key.node).is_some_and(|&epoch| key.epoch < epoch) {
            return Err("replica request belongs to fenced epoch");
        }
        if let Some(existing) = self.requests.get(&key) {
            if existing.expires_at_ms > now_ms {
                if existing.fingerprint != payload_fingerprint {
                    return Err("conflicting replayed replica request");
                }
                self.last_time_ms = now_ms;
                return Ok(Admission::Replayed);
            }
        }
        self.requests.retain(|_, entry| entry.expires_at_ms > now_ms);
        if self.requests.len() >= MAX_REQUESTS
            || (self.highest_epoch.len() >= MAX_REQUESTS
                && !self.highest_epoch.contains_key(&key.node))
        {
            return Err("replica idempotency ledger is full");
        }
        self.requests.insert(key.clone(), Entry { fingerprint: payload_fingerprint, expires_at_ms: expiry });
        self.highest_epoch.entry(key.node).and_modify(|epoch| *epoch = (*epoch).max(key.epoch))
            .or_insert(key.epoch);
        self.last_time_ms = now_ms;
        Ok(Admission::New)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(epoch: u64, nonce: u128) -> RequestKey {
        RequestKey { node: "replica".into(), epoch, nonce }
    }
    #[test]
    fn duplicate_retry_is_idempotent_but_conflicting_payload_fails() {
        let mut ledger = IdempotencyLedger::new();
        assert_eq!(ledger.admit(key(1, 1), 10, 100, 100).unwrap(), Admission::New);
        assert_eq!(ledger.admit(key(1, 1), 10, 101, 100).unwrap(), Admission::Replayed);
        assert!(ledger.admit(key(1, 1), 99, 102, 100).is_err());
    }
    #[test]
    fn advancing_epoch_fences_old_retries_even_after_expiry() {
        let mut ledger = IdempotencyLedger::new();
        ledger.admit(key(1, 1), 10, 100, 10).unwrap();
        ledger.admit(key(2, 2), 20, 120, 10).unwrap();
        assert!(ledger.admit(key(1, 1), 10, 121, 10).is_err());
        assert!(ledger.admit(key(2, 3), 30, 119, 10).is_err());
    }
    #[test]
    fn ttl_overflow_is_rejected() {
        let mut ledger = IdempotencyLedger::new();
        assert!(ledger.admit(key(1, 1), 1, u64::MAX, 2).is_err());
    }
}

#[cfg(test)]
mod full_fencing_history_tests {
    use super::*;

    #[test]
    fn existing_nodes_can_continue_after_the_history_reaches_capacity() {
        let mut ledger = IdempotencyLedger::new();
        for index in 0..MAX_REQUESTS {
            let key = RequestKey {
                node: format!("replica_{index}"),
                epoch: 1,
                nonce: 1,
            };
            assert_eq!(ledger.admit(key, 10, 100, 1).unwrap(), Admission::New);
        }

        // Request entries expire, but fencing history stays full.
        let existing = RequestKey {
            node: "replica_0".into(),
            epoch: 1,
            nonce: 2,
        };
        assert_eq!(ledger.admit(existing, 20, 101, 1).unwrap(), Admission::New);
        let unknown = RequestKey {
            node: "new_replica".into(),
            epoch: 1,
            nonce: 1,
        };
        assert!(ledger.admit(unknown, 20, 101, 1).is_err());
    }
}

#[cfg(test)]
mod idempotency_batch_tests {
    use super::*;

    fn key(epoch: u64, nonce: u128) -> RequestKey {
        RequestKey { node: "replica".into(), epoch, nonce }
    }

    #[test]
    fn conflicting_replay_does_not_commit_partial_batch_state() {
        let mut ledger = IdempotencyLedger::new();
        let batch = [(key(1, 1), 10), (key(1, 1), 11)];
        assert!(ledger.admit_batch(&batch, 100, 10).is_err());
        assert!(ledger.requests.is_empty());
        assert!(ledger.highest_epoch.is_empty());
        assert_eq!(ledger.last_time_ms, 0);

        let admitted = ledger.admit_batch(&[
            (key(1, 1), 10), (key(1, 1), 10), (key(2, 2), 20),
        ], 100, 10).unwrap();
        assert_eq!(admitted, [Admission::New, Admission::Replayed, Admission::New]);
        assert!(ledger.admit(key(1, 3), 10, 101, 10).is_err());
    }

    #[test]
    fn invalid_final_request_cannot_advance_batch_epoch() {
        let mut ledger = IdempotencyLedger::new();
        ledger.admit(key(1, 1), 10, 100, 20).unwrap();
        let before_time = ledger.last_time_ms;
        assert!(ledger.admit_batch(&[
            (key(2, 2), 20), (key(0, 3), 30),
        ], 110, 20).is_err());
        assert_eq!(ledger.last_time_ms, before_time);
        assert_eq!(ledger.highest_epoch["replica"], 1);
        assert_eq!(ledger.admit(key(1, 3), 30, 105, 20).unwrap(), Admission::New);
    }
}
