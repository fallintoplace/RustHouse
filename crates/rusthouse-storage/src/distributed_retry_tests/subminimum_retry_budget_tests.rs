    use super::*;

    #[test]
    fn early_exit_preserves_configuration_validation() {
        for seed in [0, 17, u64::MAX] {
            assert!(
                plan_retry_delays(33, 100, 1000, seed, Duration::from_millis(49))
                    .unwrap()
                    .is_empty()
            );
        }
        assert!(plan_retry_delays(1, 0, 1000, 0, Duration::ZERO).is_err());
        assert!(plan_retry_delays(34, 100, 1000, 0, Duration::ZERO).is_err());
        let first = retry_delay(0, 100, 1000, 17).unwrap();
        assert_eq!(
            plan_retry_delays(1, 100, 1000, 17, first).unwrap(),
            vec![first]
        );
    }
