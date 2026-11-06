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
