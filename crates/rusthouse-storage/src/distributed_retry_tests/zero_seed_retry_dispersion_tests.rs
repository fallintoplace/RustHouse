use super::*;
use std::collections::HashSet;

#[test]
fn zero_seed_jitters_independent_attempts_after_backoff_caps() {
    let mut delays = HashSet::new();
    for attempt in 10..=32 {
        let delay = retry_delay(attempt, 100, 1000, 0).unwrap();
        assert!((500..=1000).contains(&(delay.as_millis() as u64)));
        assert_eq!(delay, retry_delay(attempt, 100, 1000, 0).unwrap());
        delays.insert(delay);
    }
    assert!(
        delays.len() >= 10,
        "capped attempts should not all synchronize"
    );
}

#[test]
fn a_zero_attempt_still_uses_the_replica_seed() {
    let mut delays = HashSet::new();
    for seed in 0..128 {
        delays.insert(retry_delay(0, 1000, 1000, seed).unwrap());
    }
    assert!(delays.len() > 40, "seeds should spread initial retries");
}
