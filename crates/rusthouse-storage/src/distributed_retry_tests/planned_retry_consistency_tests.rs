    use super::*;

    #[test]
    fn budgeted_plan_uses_the_same_jitter_as_individual_retries() {
        for seed in [0, 1, 17, u64::MAX] {
            let planned = plan_retry_delays(33, 125, 10_000, seed, Duration::MAX).unwrap();
            assert_eq!(planned.len(), 33);
            for (attempt, delay) in planned.iter().enumerate() {
                assert_eq!(
                    *delay,
                    retry_delay(attempt as u32, 125, 10_000, seed).unwrap(),
                    "seed={seed}, attempt={attempt}"
                );
            }
            let budget = planned[..4].iter().copied().sum::<Duration>();
            let limited = plan_retry_delays(33, 125, 10_000, seed, budget).unwrap();
            assert_eq!(limited, planned[..4]);
        }
    }
