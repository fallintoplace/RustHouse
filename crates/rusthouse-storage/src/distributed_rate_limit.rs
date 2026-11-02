//! Exact, bounded replica-transfer bandwidth accounting.
//! This is deterministic planning state, not a network traffic shaper.
//! Persist checkpoints atomically with verified transfer progress.

/// Each credit represents one byte; fractional refill is kept in
/// thousandths of a byte for millisecond timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenBucket {
    capacity: u64,
    tokens: u64,
    rate_bytes_per_second: u64,
    last_ms: u64,
    remainder: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantedTransfer {
    pub bytes: u64,
    pub bucket: TokenBucket,
}

impl TokenBucket {
    /// Start a new limiter with a full burst allowance.
    pub fn new(
        capacity: u64,
        rate_bytes_per_second: u64,
        at_ms: u64,
    ) -> Result<Self, &'static str> {
        Self::restore(capacity, capacity, rate_bytes_per_second, at_ms, 0)
    }

    /// Restore exact checkpoint state after a process restart.
    ///
    /// A checkpoint must be trusted and durably coordinated with transferred
    /// bytes. An arbitrary caller must not manufacture extra available tokens.
    pub fn restore(
        capacity: u64,
        tokens: u64,
        rate_bytes_per_second: u64,
        last_ms: u64,
        remainder: u16,
    ) -> Result<Self, &'static str> {
        if capacity == 0
            || rate_bytes_per_second == 0
            || tokens > capacity
            || remainder >= 1000
            || (tokens == capacity && remainder != 0)
        {
            return Err("invalid replica token-bucket checkpoint");
        }
        Ok(Self {
            capacity,
            tokens,
            rate_bytes_per_second,
            last_ms,
            remainder,
        })
    }

    pub fn capacity(self) -> u64 {
        self.capacity
    }

    pub fn available(self) -> u64 {
        self.tokens
    }

    pub fn timestamp_ms(self) -> u64 {
        self.last_ms
    }

    pub fn fractional_credit(self) -> u16 {
        self.remainder
    }

    /// Refill using exact unsigned integer arithmetic.
    ///
    /// A cap-safe fast path handles enormous elapsed times without even
    /// multiplying two potentially maximal u64 inputs. Credits accumulated
    /// while already full are discarded, including fractional credits.
    pub fn refill(self, now_ms: u64) -> Result<Self, &'static str> {
        let elapsed = now_ms
            .checked_sub(self.last_ms)
            .ok_or("replica token-bucket clock moved backwards")?;
        if elapsed == 0 {
            return Ok(self);
        }
        if self.tokens == self.capacity {
            return Ok(Self {
                last_ms: now_ms,
                remainder: 0,
                ..self
            });
        }

        // The maximum missing capacity is u64::MAX bytes, so this
        // threshold fits comfortably in u128.
        let needed = u128::from(self.capacity - self.tokens) * 1000;
        let threshold = needed.div_ceil(u128::from(self.rate_bytes_per_second));
        if u128::from(elapsed) >= threshold {
            return Ok(Self {
                tokens: self.capacity,
                last_ms: now_ms,
                remainder: 0,
                ..self
            });
        }

        // Before the threshold, elapsed * rate < capacity * 1000.
        // This is bounded in u128 even for maximum u64 inputs.
        let earned = u128::from(elapsed)
            * u128::from(self.rate_bytes_per_second)
            + u128::from(self.remainder);
        let bytes = (earned / 1000) as u64;
        let remainder = (earned % 1000) as u16;
        let tokens = self.tokens + bytes;
        if tokens >= self.capacity {
            Ok(Self {
                tokens: self.capacity,
                last_ms: now_ms,
                remainder: 0,
                ..self
            })
        } else {
            Ok(Self {
                tokens,
                last_ms: now_ms,
                remainder,
                ..self
            })
        }
    }

    /// Reserve at most the requested number of bytes for a planned transfer.
    ///
    /// A zero-byte grant still advances the refill timestamp, and a
    /// repeated grant at the same timestamp cannot create new credits.
    /// Commit the returned bucket only with the corresponding verified
    /// transfer progress.
    pub fn grant(self, now_ms: u64, requested: u64) -> Result<GrantedTransfer, &'static str> {
        let mut bucket = self.refill(now_ms)?;
        let bytes = bucket.tokens.min(requested);
        bucket.tokens -= bytes;
        Ok(GrantedTransfer { bytes, bucket })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refill_preserves_fractional_bytes_across_small_intervals() {
        let initial = TokenBucket::new(10, 1, 1000).unwrap();
        let granted = initial.grant(1000, 10).unwrap();
        assert_eq!(granted.bytes, 10);
        assert_eq!(granted.bucket.available(), 0);

        let mut bucket = granted.bucket;
        for millis in 1001..2000 {
            bucket = bucket.refill(millis).unwrap();
            assert_eq!(bucket.available(), 0);
        }
        assert_eq!(bucket.fractional_credit(), 999);
        let bucket = bucket.refill(2000).unwrap();
        assert_eq!(bucket.available(), 1);
        assert_eq!(bucket.fractional_credit(), 0);
    }

    #[test]
    fn repeated_same_timestamp_never_mints_extra_bandwidth() {
        let bucket = TokenBucket::new(20, 1000, 0).unwrap();
        let first = bucket.grant(0, 19).unwrap();
        let second = first.bucket.grant(0, 20).unwrap();
        let third = second.bucket.grant(0, 20).unwrap();
        assert_eq!((first.bytes, second.bytes, third.bytes), (19, 1, 0));
        assert_eq!(third.bucket.available(), 0);
        let next = third.bucket.grant(5, 20).unwrap();
        assert_eq!(next.bytes, 5);
        assert_eq!(next.bucket.available(), 0);
    }

    #[test]
    fn full_bucket_discards_time_and_fractional_credit() {
        let bucket = TokenBucket::new(5, 100, 0).unwrap();
        let at_999 = bucket.refill(999).unwrap();
        assert_eq!(at_999.available(), 5);
        assert_eq!(at_999.fractional_credit(), 0);
        let drained = at_999.grant(999, 5).unwrap();
        assert_eq!(drained.bucket.refill(1000).unwrap().available(), 0);
        assert_eq!(drained.bucket.refill(1009).unwrap().available(), 1);
    }

    #[test]
    fn rejects_invalid_checkpoints_and_nonmonotonic_time() {
        assert!(TokenBucket::new(0, 1, 0).is_err());
        assert!(TokenBucket::new(1, 0, 0).is_err());
        assert!(TokenBucket::restore(10, 11, 1, 0, 0).is_err());
        assert!(TokenBucket::restore(10, 10, 1, 0, 1).is_err());
        assert!(TokenBucket::restore(10, 0, 1, 0, 1000).is_err());
        let bucket = TokenBucket::restore(10, 4, 1, 100, 500).unwrap();
        assert!(bucket.refill(99).is_err());
        assert_eq!(bucket, TokenBucket::restore(10, 4, 1, 100, 500).unwrap());
    }

    #[test]
    fn maximum_inputs_saturate_at_capacity_without_overflow() {
        let bucket = TokenBucket::new(u64::MAX, u64::MAX, 0).unwrap();
        let empty = bucket.grant(0, u64::MAX).unwrap().bucket;
        let restored = empty.refill(u64::MAX).unwrap();
        assert_eq!(restored.available(), u64::MAX);
        assert_eq!(restored.timestamp_ms(), u64::MAX);
        assert_eq!(restored.fractional_credit(), 0);
        let slow = TokenBucket::restore(u64::MAX, 0, 1, 0, 999).unwrap();
        let after = slow.refill(u64::MAX).unwrap();
        assert_eq!(after.available(), u64::MAX / 1000 + 1);
        assert_eq!(after.fractional_credit(), 614);
    }

    #[test]
    fn restored_checkpoint_matches_uninterrupted_accounting() {
        let bucket = TokenBucket::new(100, 33, 40).unwrap();
        let grant = bucket.grant(40, 100).unwrap();
        let half = grant.bucket.refill(550).unwrap();
        let restored = TokenBucket::restore(
            half.capacity(),
            half.available(),
            33,
            half.timestamp_ms(),
            half.fractional_credit(),
        )
        .unwrap();
        let uninterrupted = half.grant(1100, 100).unwrap();
        let resumed = restored.grant(1100, 100).unwrap();
        assert_eq!(uninterrupted, resumed);
    }
}
