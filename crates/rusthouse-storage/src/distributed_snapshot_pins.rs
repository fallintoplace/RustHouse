//! Bounded local reader pins for conservative snapshot retention.
//! Tokens are issued by an external authority; this is not consensus.
use crate::distributed_fencing::FenceToken;
use crate::distributed_lineage::SnapshotVersion;
use std::collections::BTreeMap;

const MAX_PINS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderPin {
    pub reader: String,
    pub snapshot: SnapshotVersion,
    pub token: FenceToken,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ReaderPinLedger {
    active: BTreeMap<String, ReaderPin>,
    highest: BTreeMap<String, FenceToken>,
    last_clock_ms: u64,
}

fn valid_reader(reader: &str) -> bool {
    !reader.is_empty() && reader.len() <= 64
        && reader.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

impl ReaderPinLedger {
    pub fn new() -> Self { Self::default() }

    /// A caller must pin before using a snapshot. An identical token can
    /// renew only the same reader's snapshot and never shorten its expiry.
    pub fn pin(&mut self, pin: ReaderPin, now_ms: u64, ttl_limit: u64)
        -> Result<(), &'static str>
    {
        if !valid_reader(&pin.reader)
            || pin.snapshot.epoch == 0 || pin.snapshot.generation == 0
            || pin.token.epoch == 0 || pin.token.sequence == 0
            || now_ms < self.last_clock_ms || ttl_limit == 0
            || pin.expires_at_ms <= now_ms
            || pin.expires_at_ms > now_ms.saturating_add(ttl_limit)
        {
            return Err("invalid reader snapshot pin or clock");
        }
        if let Some(&previous) = self.highest.get(&pin.reader) {
            if pin.token < previous { return Err("stale reader fencing token"); }
        }
        if let Some(previous) = self.active.get(&pin.reader) {
            if pin.token == previous.token
                && (pin.snapshot != previous.snapshot || pin.expires_at_ms < previous.expires_at_ms)
            {
                return Err("conflicting reader pin renewal");
            }
        }
        if !self.active.contains_key(&pin.reader) && self.active.len() >= MAX_PINS {
            return Err("reader pin ledger is full");
        }
        if !self.highest.contains_key(&pin.reader) && self.highest.len() >= MAX_PINS {
            return Err("reader fence history is full");
        }
        self.last_clock_ms = now_ms;
        self.highest.insert(pin.reader.clone(), pin.token);
        self.active.insert(pin.reader.clone(), pin);
        Ok(())
    }

    /// Preserve high-water fencing history after lease expiry. Calling code
    /// must not use absence of a pin to infer remote readers have exited.
    pub fn expire(&mut self, now_ms: u64) -> Result<usize, &'static str> {
        if now_ms < self.last_clock_ms { return Err("reader pin clock moved backwards"); }
        self.last_clock_ms = now_ms;
        let before = self.active.len();
        self.active.retain(|_, pin| pin.expires_at_ms > now_ms);
        Ok(before - self.active.len())
    }

    pub fn pinned_versions(&self, now_ms: u64) -> Vec<SnapshotVersion> {
        let mut versions = self.active.values()
            .filter(|pin| pin.expires_at_ms > now_ms)
            .map(|pin| pin.snapshot).collect::<Vec<_>>();
        versions.sort_unstable_by_key(|v| (v.epoch, v.generation, v.fingerprint));
        versions.dedup();
        versions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pin(reader: &str, token: u64, generation: u64, expiry: u64) -> ReaderPin {
        ReaderPin {
            reader: reader.into(),
            snapshot: SnapshotVersion { epoch: 1, generation, fingerprint: generation as u128 },
            token: FenceToken { epoch: 1, sequence: token },
            expires_at_ms: expiry,
        }
    }

    #[test]
    fn old_reader_cannot_reclaim_lease_after_expiry() {
        let mut ledger = ReaderPinLedger::new();
        ledger.pin(pin("reader", 8, 2, 200), 100, 100).unwrap();
        assert_eq!(ledger.pinned_versions(150).len(), 1);
        assert_eq!(ledger.expire(200).unwrap(), 1);
        assert!(ledger.pin(pin("reader", 7, 2, 300), 201, 100).is_err());
        ledger.pin(pin("reader", 9, 3, 300), 201, 100).unwrap();
        assert_eq!(ledger.pinned_versions(205)[0].generation, 3);
    }

    #[test]
    fn renewal_cannot_change_snapshot_without_new_token() {
        let mut ledger = ReaderPinLedger::new();
        ledger.pin(pin("reader", 8, 2, 200), 100, 100).unwrap();
        assert!(ledger.pin(pin("reader", 8, 3, 201), 101, 100).is_err());
        assert!(ledger.pin(pin("reader", 8, 2, 199), 101, 100).is_err());
        ledger.pin(pin("reader", 8, 2, 201), 101, 100).unwrap();
    }

    #[test]
    fn failed_clock_regression_preserves_active_pins() {
        let mut ledger = ReaderPinLedger::new();
        ledger.pin(pin("reader", 1, 1, 110), 100, 10).unwrap();
        assert!(ledger.expire(99).is_err());
        assert_eq!(ledger.pinned_versions(101).len(), 1);
    }
}
