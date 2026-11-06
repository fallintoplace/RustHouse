use super::*;

#[test]
fn resumed_schedule_matches_the_exact_absolute_attempt_jitter() {
    for seed in [0, 7, 97, u64::MAX] {
        let all = plan_retry_delays(33, 100, 8_000, seed, Duration::MAX).unwrap();
        for first in [0, 1, 5, 16, 32, 33] {
            let resumed =
                plan_retry_delays_from(first, 33 - first, 100, 8_000, seed, Duration::MAX)
                    .unwrap();
            assert_eq!(resumed, all[first as usize..]);
            let budget = resumed.iter().take(3).copied().sum::<Duration>();
            let partial =
                plan_retry_delays_from(first, 33 - first, 100, 8_000, seed, budget).unwrap();
            assert_eq!(partial, resumed[..resumed.len().min(3)]);
        }
    }
}

#[test]
fn rejects_invalid_attempt_windows_even_for_empty_deadlines() {
    for (start, count) in [(33, 1), (32, 2), (34, 0), (u32::MAX, 0)] {
        assert!(plan_retry_delays_from(start, count, 1, 2, 0, Duration::ZERO).is_err());
    }
    assert!(plan_retry_delays_from(33, 0, 0, 1, 0, Duration::ZERO).is_err());
    assert!(
        plan_retry_delays_from(0, 0, 1, 1, 0, Duration::ZERO)
            .unwrap()
            .is_empty()
    );
    assert!(
        plan_retry_delays_from(32, 1, 1, 1, 0, Duration::ZERO)
            .unwrap()
            .is_empty()
    );
}
