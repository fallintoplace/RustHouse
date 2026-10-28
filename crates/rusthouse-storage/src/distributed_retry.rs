//! Deterministic retry delays for a future replica transfer scheduler.
use std::time::Duration;

/// Cap exponential retries at max_ms; jitter is stable for the supplied seed.
pub fn retry_delay(
    attempt: u32,
    base_ms: u64,
    max_ms: u64,
    seed: u64,
) -> Result<Duration, &'static str> {
    if attempt > 32 || base_ms == 0 || max_ms == 0 || base_ms > max_ms {
        return Err("invalid replica retry configuration");
    }
    let exponent = base_ms.saturating_mul(1u64 << attempt).min(max_ms);
    let half = exponent / 2;
    let jitter = seed
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .rotate_left(attempt)
        % (exponent - half + 1);
    Ok(Duration::from_millis(half + jitter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_exceeds_deadline_even_with_large_attempts() {
        for attempt in 0..=32 {
            let delay = retry_delay(attempt, 100, 5_000, 1234).unwrap();
            assert!(delay.as_millis() > 0);
            assert!(delay.as_millis() <= 5_000);
        }
    }

    #[test]
    fn rejects_zero_and_bad_bounds() {
        assert!(retry_delay(0, 0, 100, 1).is_err());
        assert!(retry_delay(0, 100, 10, 1).is_err());
        assert!(retry_delay(33, 1, 10, 1).is_err());
    }
}