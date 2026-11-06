use super::*;

#[test]
fn opens_after_threshold_and_exponentially_retries_single_probes() {
    let mut breaker = ReplicaCircuitBreaker::new(2, 10, 40, 100).unwrap();
    assert!(breaker.permit(100).unwrap());
    breaker.record_failure(100).unwrap();
    assert_eq!(breaker.phase(), ReplicaCircuitPhase::Closed);
    assert!(breaker.permit(101).unwrap());
    breaker.record_failure(101).unwrap();
    assert_eq!(breaker.retry_at_ms(), Some(111));
    assert!(!breaker.permit(110).unwrap());
    assert!(breaker.permit(111).unwrap());
    assert_eq!(breaker.phase(), ReplicaCircuitPhase::HalfOpen);
    assert!(!breaker.permit(111).unwrap());
    breaker.record_failure(111).unwrap();
    assert_eq!(breaker.retry_at_ms(), Some(131));
    assert!(!breaker.permit(130).unwrap());
    assert!(breaker.permit(131).unwrap());
    breaker.record_success(131).unwrap();
    assert_eq!(breaker.phase(), ReplicaCircuitPhase::Closed);
    assert_eq!(breaker.retry_at_ms(), None);
}

#[test]
fn rejects_backwards_clock_and_overflow_without_mutation() {
    let mut breaker = ReplicaCircuitBreaker::new(1, 10, 40, 100).unwrap();
    let before = breaker;
    assert!(breaker.record_failure(99).is_err());
    assert_eq!(breaker, before);
    assert!(breaker.permit(99).is_err());
    assert_eq!(breaker, before);

    let mut end = ReplicaCircuitBreaker::new(1, 10, 40, u64::MAX - 5).unwrap();
    let before = end;
    assert!(end.record_failure(u64::MAX - 5).is_err());
    assert_eq!(end, before);
    assert!(ReplicaCircuitBreaker::new(0, 10, 40, 0).is_err());
    assert!(ReplicaCircuitBreaker::new(1, 40, 10, 0).is_err());

    // A checked bit shift does not detect truncated high bits. The
    // second backoff must saturate and reject an unrepresentable deadline.
    let mut huge = ReplicaCircuitBreaker::new(1, 1u64 << 63, u64::MAX, 0).unwrap();
    huge.record_failure(0).unwrap();
    assert_eq!(huge.retry_at_ms(), Some(1u64 << 63));
    assert!(huge.permit(1u64 << 63).unwrap());
    let unchanged = huge;
    assert!(huge.record_failure(1u64 << 63).is_err());
    assert_eq!(huge, unchanged);
}
