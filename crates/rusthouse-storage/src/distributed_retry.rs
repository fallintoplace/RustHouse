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
    let half = exponent.div_ceil(2);
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
    fn one_millisecond_backoff_never_becomes_zero() {
        for seed in [0, 1, u64::MAX] {
            assert_eq!(
                retry_delay(0, 1, 1, seed).unwrap(),
                Duration::from_millis(1)
            );
        }
    }

    #[test]
    fn rejects_zero_and_bad_bounds() {
        assert!(retry_delay(0, 0, 100, 1).is_err());
        assert!(retry_delay(0, 100, 10, 1).is_err());
        assert!(retry_delay(33, 1, 10, 1).is_err());
    }
    #[test]
    fn retry_backoff_is_deterministic_at_integer_boundaries() {
        for attempt in 0..=32 {
            for seed in [0, 1, u64::MAX] {
                let delay = retry_delay(attempt, u64::MAX, u64::MAX, seed).unwrap();
                assert!(delay >= Duration::from_millis(1));
                assert!(delay.as_millis() <= u128::from(u64::MAX));
                assert_eq!(
                    delay,
                    retry_delay(attempt, u64::MAX, u64::MAX, seed).unwrap()
                );
            }
        }
    }
}

/// Return no scheduled retry when it cannot fit within the remaining deadline.
pub fn retry_delay_with_deadline(
    attempt: u32,
    base_ms: u64,
    max_ms: u64,
    seed: u64,
    remaining: Duration,
) -> Result<Option<Duration>, &'static str> {
    let delay = retry_delay(attempt, base_ms, max_ms, seed)?;
    Ok((delay <= remaining).then_some(delay))
}

#[cfg(test)]
mod retry_deadline_tests {
    use super::*;

    #[test]
    fn skips_retries_that_outlive_the_budget() {
        let delay = retry_delay(3, 100, 1_000, 7).unwrap();
        assert_eq!(
            retry_delay_with_deadline(3, 100, 1_000, 7, delay).unwrap(),
            Some(delay)
        );
        assert_eq!(
            retry_delay_with_deadline(3, 100, 1_000, 7, Duration::ZERO).unwrap(),
            None
        );
        assert!(retry_delay_with_deadline(33, 100, 1_000, 7, delay).is_err());
    }
}

/// Precompute up to 33 bounded retry delays without exceeding the total budget.
/// This only schedules delays and never starts network transfers.
pub fn plan_retry_delays(
    attempts: u32,
    base_ms: u64,
    max_ms: u64,
    seed: u64,
    budget: Duration,
) -> Result<Vec<Duration>, &'static str> {
    if attempts > 33 {
        return Err("retry count exceeds supported attempt range");
    }
    // Validate settings even if the caller requests no retries.
    retry_delay(0, base_ms, max_ms, seed)?;
    let mut elapsed = Duration::ZERO;
    let mut delays = Vec::with_capacity(attempts as usize);
    for attempt in 0..attempts {
        let delay = retry_delay(attempt, base_ms, max_ms, seed.wrapping_add(u64::from(attempt)))?;
        if delay > budget.saturating_sub(elapsed) {
            break;
        }
        elapsed += delay;
        delays.push(delay);
    }
    Ok(delays)
}

#[cfg(test)]
mod total_retry_budget_tests {
    use super::*;

    #[test]
    fn caps_cumulative_retry_delay_and_is_deterministic() {
        let budget = Duration::from_millis(250);
        let scheduled = plan_retry_delays(33, 100, 1000, 2, budget).unwrap();
        assert!(!scheduled.is_empty());
        assert!(scheduled.iter().copied().sum::<Duration>() <= budget);
        assert_eq!(scheduled, plan_retry_delays(33, 100, 1000, 2, budget).unwrap());
        assert!(plan_retry_delays(34, 1, 10, 0, budget).is_err());
        assert!(plan_retry_delays(0, 0, 10, 0, budget).is_err());
        assert!(plan_retry_delays(5, 100, 1000, 1, Duration::ZERO).unwrap().is_empty());
    }
}
