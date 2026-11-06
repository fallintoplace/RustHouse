use super::*;

#[test]
fn reserves_only_within_query_budget_and_does_not_overcount_failures() {
    let bytes = AtomicUsize::new(0);
    assert!(reserve_shard_bytes(&bytes, MAX_SHARD_INPUT_BYTES - 10).is_ok());
    assert!(reserve_shard_bytes(&bytes, 11).is_err());
    assert_eq!(
        bytes.load(AtomicOrdering::Relaxed),
        MAX_SHARD_INPUT_BYTES - 10
    );
    assert!(reserve_shard_bytes(&bytes, 10).is_ok());
    assert_eq!(bytes.load(AtomicOrdering::Relaxed), MAX_SHARD_INPUT_BYTES);
}

#[test]
fn reservations_cannot_exceed_budget_under_contention() {
    let bytes = AtomicUsize::new(0);
    let succeeded = std::thread::scope(|scope| {
        let tasks: Vec<_> = (0..16)
            .map(|_| scope.spawn(|| reserve_shard_bytes(&bytes, 8 * 1024 * 1024).is_ok()))
            .collect();
        tasks
            .into_iter()
            .map(|task| task.join().unwrap())
            .filter(|&ok| ok)
            .count()
    });
    assert_eq!(succeeded, 8);
    assert_eq!(bytes.load(AtomicOrdering::Relaxed), MAX_SHARD_INPUT_BYTES);
}
