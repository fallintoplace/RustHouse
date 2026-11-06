use super::*;

#[test]
fn does_not_reclaim_generations_a_replica_still_needs() {
    assert_eq!(
        retention_candidates_below_replica_floor(&[1, 2, 3, 4, 5], 5, 2, 3).unwrap(),
        vec![1, 2]
    );
    assert_eq!(
        retention_candidates_below_replica_floor(&[1, 2, 3, 4], 4, 1, 1).unwrap(),
        Vec::<u64>::new()
    );
    assert!(retention_candidates_below_replica_floor(&[1, 2], 2, 1, 0).is_err());
}
