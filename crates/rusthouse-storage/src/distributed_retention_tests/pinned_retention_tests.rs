use super::*;

#[test]
fn single_pin_avoids_indexing_and_preserves_retention_order() {
    let generations = (1..=32_u64).rev().collect::<Vec<_>>();
    let candidates = retention_candidates_with_pins(&generations, 32, 2, &[3]).unwrap();
    assert_eq!(candidates.first(), Some(&1));
    assert!(!candidates.contains(&3));
    assert_eq!(candidates.last(), Some(&30));
    assert!(retention_candidates_with_pins(&generations, 32, 2, &[99]).is_err());
}

#[test]
fn preserves_backup_pins_and_active_generation() {
    assert_eq!(
        retention_candidates_with_pins(&[1, 2, 3, 4, 5], 5, 2, &[1, 3]).unwrap(),
        vec![2]
    );
    assert_eq!(
        retention_candidates_with_pins(&[1, 2, 3], 1, 1, &[2]).unwrap(),
        Vec::<u64>::new()
    );
}

#[test]
fn no_pins_use_the_same_retention_plan_as_unpinned_reads() {
    let generations = (1..=16_384_u64).rev().collect::<Vec<_>>();
    for active in [1u64, 8_192, 16_384] {
        let expected = retention_candidates(&generations, active, 128).unwrap();
        assert_eq!(
            retention_candidates_with_pins(&generations, active, 128, &[]).unwrap(),
            expected,
        );
    }
    assert!(retention_candidates_with_pins(&[1, 1], 1, 1, &[]).is_err());
}

#[test]
fn rejects_stale_and_duplicate_pins() {
    assert!(retention_candidates_with_pins(&[1, 2, 3], 3, 1, &[4]).is_err());
    assert!(retention_candidates_with_pins(&[1, 2, 3], 3, 1, &[1, 1]).is_err());
    assert!(retention_candidates_with_pins(&[1, 1], 1, 1, &[]).is_err());
}
