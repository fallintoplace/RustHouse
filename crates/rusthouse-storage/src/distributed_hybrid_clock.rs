//! Bounded hybrid logical timestamps for replica metadata.
//! A timestamp is a causality hint, not a consensus or lease certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct HybridTimestamp {
    pub physical_ms: u64,
    pub logical: u32,
    pub node: u16,
}

#[derive(Clone, Debug)]
pub struct HybridClock {
    current: HybridTimestamp,
}

impl HybridClock {
    pub fn new(node: u16, now_ms: u64) -> Self {
        Self { current: HybridTimestamp { physical_ms: now_ms, logical: 0, node } }
    }


    /// Resume from externally durable clock state after a restart.
    /// The caller must persist timestamps before publishing their effects.
    pub fn restore(
        node: u16,
        persisted: HybridTimestamp,
        now_ms: u64,
        max_skew_ms: u64,
    ) -> Result<Self, &'static str> {
        if persisted.node != node {
            return Err("hybrid clock persisted node mismatch");
        }
        if persisted.physical_ms > now_ms.saturating_add(max_skew_ms) {
            return Err("persisted hybrid clock exceeds allowed clock skew");
        }
        Ok(Self { current: persisted })
    }

    pub fn current(&self) -> HybridTimestamp { self.current }

    /// Advance even when the local wall clock moves backwards.
    pub fn tick(&mut self, now_ms: u64) -> Result<HybridTimestamp, &'static str> {
        let next = if now_ms > self.current.physical_ms {
            HybridTimestamp { physical_ms: now_ms, logical: 0, node: self.current.node }
        } else {
            HybridTimestamp {
                physical_ms: self.current.physical_ms,
                logical: self.current.logical.checked_add(1).ok_or("hybrid clock logical overflow")?,
                node: self.current.node,
            }
        };
        self.current = next;
        Ok(next)
    }

    /// Receive a remote event using the standard max-physical-time HLC rules.
    /// Reject implausibly future remote timestamps before changing local state.
    pub fn observe(
        &mut self,
        remote: HybridTimestamp,
        now_ms: u64,
        max_skew_ms: u64,
    ) -> Result<HybridTimestamp, &'static str> {
        if remote.physical_ms > now_ms.saturating_add(max_skew_ms) {
            return Err("remote hybrid clock exceeds allowed clock skew");
        }
        let physical = self.current.physical_ms.max(remote.physical_ms).max(now_ms);
        let logical = if physical == self.current.physical_ms && physical == remote.physical_ms {
            self.current.logical.max(remote.logical)
                .checked_add(1).ok_or("hybrid clock logical overflow")?
        } else if physical == self.current.physical_ms {
            self.current.logical.checked_add(1).ok_or("hybrid clock logical overflow")?
        } else if physical == remote.physical_ms {
            remote.logical.checked_add(1).ok_or("hybrid clock logical overflow")?
        } else {
            0
        };
        let next = HybridTimestamp { physical_ms: physical, logical, node: self.current.node };
        self.current = next;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_rollback_keeps_monotonic_order() {
        let mut clock = HybridClock::new(7, 100);
        let a = clock.tick(101).unwrap();
        let b = clock.tick(90).unwrap();
        assert!(b > a);
        assert_eq!((b.physical_ms, b.logical), (101, 1));
    }

    #[test]
    fn observing_same_time_advances_past_both_logical_counters() {
        let mut clock = HybridClock::new(2, 100);
        clock.tick(100).unwrap();
        let remote = HybridTimestamp { physical_ms: 100, logical: 9, node: 3 };
        assert_eq!(clock.observe(remote, 99, 2).unwrap().logical, 10);
        assert_eq!(clock.observe(remote, 102, 2).unwrap().logical, 0);
    }

    #[test]
    fn skew_error_is_atomic() {
        let mut clock = HybridClock::new(1, 100);
        let before = clock.current();
        let remote = HybridTimestamp { physical_ms: 1000, logical: 0, node: 2 };
        assert!(clock.observe(remote, 100, 20).is_err());
        assert_eq!(clock.current(), before);
    }

    #[test]
    fn remote_logical_overflow_does_not_advance_state() {
        let mut clock = HybridClock::new(1, 1);
        let before = clock.current();
        let remote = HybridTimestamp { physical_ms: 2, logical: u32::MAX, node: 3 };
        assert!(clock.observe(remote, 2, 0).is_err());
        assert_eq!(clock.current(), before);
    }
}

#[cfg(test)]
mod restart_clock_tests {
    use super::*;

    #[test]
    fn restored_clock_remains_monotonic_across_wall_clock_rollback() {
        let mut original = HybridClock::new(7, 100);
        let previous = original.tick(100).unwrap();
        let mut restored = HybridClock::restore(7, previous, 90, 20).unwrap();
        assert_eq!(restored.current(), previous);
        assert!(restored.tick(90).unwrap() > previous);
        assert_eq!(restored.current().logical, 2);
    }

    #[test]
    fn wrong_node_and_excessive_clock_skew_fail_closed() {
        let previous = HybridTimestamp { node: 7, physical_ms: 100, logical: 5 };
        assert!(HybridClock::restore(8, previous, 100, 10).is_err());
        assert!(HybridClock::restore(7, previous, 70, 20).is_err());
        assert_eq!(HybridClock::restore(7, previous, 70, 30).unwrap().current(), previous);
    }
}
