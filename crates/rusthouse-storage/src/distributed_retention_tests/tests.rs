use super::*;

#[test]
fn preserves_current_and_recent_generations() {
    assert_eq!(retention_candidates(&[4, 1, 3, 2], 2, 2).unwrap(), vec![1]);
    assert!(retention_candidates(&[1, 2], 2, 5).unwrap().is_empty());
}

#[test]
fn rejects_unknown_or_duplicate_active_generation() {
    assert!(retention_candidates(&[1, 1, 2], 1, 1).is_err());
    assert!(retention_candidates(&[1, 2], 3, 1).is_err());
    assert!(retention_candidates(&[1, 2], 1, 0).is_err());
}
#[test]
fn protects_active_generation_even_when_it_is_oldest() {
    assert_eq!(
        retention_candidates(&[5, 3, 1, 4, 2], 1, 2).unwrap(),
        vec![2, 3]
    );
    assert_eq!(
        retention_candidates(&[5, 3, 1, 4, 2], 5, 2).unwrap(),
        vec![1, 2, 3]
    );
    assert!(
        retention_candidates(&[1, 2, 3], 1, usize::MAX)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn unsorted_generations_still_produce_sorted_retention_candidates() {
    assert_eq!(
        retention_candidates(&[12, 5, 8, 3, 20, 1], 5, 2).unwrap(),
        vec![1, 3, 8]
    );
    assert!(retention_candidates(&[3, 1, 3], 1, 1).is_err());
}
