use super::*;

#[test]
fn planned_first_delay_matches_the_individual_retry_delay() {
    for seed in [0, 1, 42, u64::MAX] {
        let expected = retry_delay(0, 100, 5000, seed).unwrap();
        assert_eq!(
            plan_retry_delays(1, 100, 5000, seed, Duration::MAX).unwrap(),
            vec![expected]
        );
    }
    assert!(plan_retry_delays(0, 0, 5000, 0, Duration::MAX).is_err());
    assert!(
        plan_retry_delays(0, 100, 5000, 0, Duration::MAX)
            .unwrap()
            .is_empty()
    );
}
