    use super::*;

    #[test]
    fn caps_cumulative_retry_delay_and_is_deterministic() {
        let budget = Duration::from_millis(250);
        let scheduled = plan_retry_delays(33, 100, 1000, 2, budget).unwrap();
        assert!(!scheduled.is_empty());
        assert!(scheduled.iter().copied().sum::<Duration>() <= budget);
        assert_eq!(
            scheduled,
            plan_retry_delays(33, 100, 1000, 2, budget).unwrap()
        );
        assert!(plan_retry_delays(34, 1, 10, 0, budget).is_err());
        assert!(plan_retry_delays(0, 0, 10, 0, budget).is_err());
        assert!(
            plan_retry_delays(5, 100, 1000, 1, Duration::ZERO)
                .unwrap()
                .is_empty()
        );
    }
