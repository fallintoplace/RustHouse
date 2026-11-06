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
