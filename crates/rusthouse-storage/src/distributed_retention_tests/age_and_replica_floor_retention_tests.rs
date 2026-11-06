use super::*;

#[test]
fn age_floor_combines_with_active_reader_pins_and_lagging_replicas() {
    let snapshots = [(5, 200), (3, 130), (1, 100), (4, 160), (2, 190)];
    assert_eq!(
        retention_candidates_with_min_age(&snapshots, 5, 1, &[3], &[5], 200, 40).unwrap(),
        vec![1, 4]
    );
    assert_eq!(
        retention_candidates_with_min_age(&snapshots, 5, 1, &[3], &[4], 200, 40).unwrap(),
        vec![1]
    );
    assert_eq!(
        retention_candidates_with_min_age(&snapshots, 5, 1, &[], &[1], 200, 0).unwrap(),
        Vec::<u64>::new()
    );
    assert_eq!(
        retention_candidates_with_min_age(&snapshots, 5, 2, &[], &[5], 200, 40).unwrap(),
        vec![1, 3]
    );
}

#[test]
fn older_minimum_age_never_expands_retention_candidates() {
    let generations = (1..=256u64).map(|id| (id, id * 3)).collect::<Vec<_>>();
    let mut previous =
        retention_candidates_with_min_age(&generations, 256, 1, &[], &[256], 768, 0).unwrap();
    for age in [1, 3, 24, 96, 384, 768, u64::MAX] {
        let candidate =
            retention_candidates_with_min_age(&generations, 256, 1, &[], &[256], 768, age)
                .unwrap();
        assert!(
            candidate
                .iter()
                .all(|generation| previous.contains(generation))
        );
        previous = candidate;
    }
}

#[test]
fn rejects_future_timestamps_stale_pins_and_missing_replica_floors() {
    let snapshots = [(1, 100), (2, 101)];
    assert!(retention_candidates_with_min_age(&snapshots, 2, 1, &[], &[2], 100, 0).is_err());
    assert!(retention_candidates_with_min_age(&snapshots, 2, 1, &[3], &[2], 101, 0).is_err());
    assert!(retention_candidates_with_min_age(&snapshots, 2, 1, &[], &[], 101, 0).is_err());
    assert!(
        retention_candidates_with_min_age(&[(1, 0), (1, 0)], 1, 1, &[], &[1], 0, 0).is_err()
    );
}
