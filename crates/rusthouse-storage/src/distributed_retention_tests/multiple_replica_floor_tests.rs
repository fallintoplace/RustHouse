use super::*;

#[test]
fn slowest_replica_and_reader_pins_are_jointly_protected() {
    let generations = [1, 2, 3, 4, 5, 6, 7];
    let selected =
        retention_candidates_for_replica_floors(&generations, 7, 2, &[2], &[5, 3, 6]).unwrap();
    assert_eq!(selected, vec![1]);
    assert_eq!(
        retention_candidates_for_replica_floors(&generations, 7, 2, &[2], &[6, 5, 3]).unwrap(),
        selected,
    );
    assert_eq!(
        retention_candidates_for_replica_floors(&generations, 7, 2, &[], &[3]).unwrap(),
        vec![1, 2],
    );
    assert_eq!(
        retention_candidates_for_replica_floors(&generations, 7, 2, &[2], &[1]).unwrap(),
        Vec::<u64>::new(),
    );
}

#[test]
fn multiple_floors_match_intersection_of_all_single_replica_plans() {
    let generations = (1..=256u64).rev().collect::<Vec<_>>();
    let pins = [2, 9, 17, 53, 120];
    for active in [1u64, 53, 256] {
        for keep in [1, 2, 8, 256, usize::MAX] {
            for floors in [&[1u64, 99, 234][..], &[2u64, 100, 255][..], &[257u64][..]] {
                let result = retention_candidates_for_replica_floors(
                    &generations,
                    active,
                    keep,
                    &pins,
                    floors,
                )
                .unwrap();
                let mut expected =
                    retention_candidates_with_pins(&generations, active, keep, &pins).unwrap();
                expected.retain(|generation| floors.iter().all(|floor| generation < floor));
                assert_eq!(result, expected);
            }
        }
    }
}

#[test]
fn fails_closed_on_missing_floors_or_stale_pins() {
    let generations = [1, 2, 3, 4];
    assert!(retention_candidates_for_replica_floors(&generations, 4, 1, &[], &[]).is_err());
    assert!(retention_candidates_for_replica_floors(&generations, 4, 1, &[], &[0]).is_err());
    assert!(
        retention_candidates_for_replica_floors(&generations, 4, 1, &[], &[1; 65]).is_err()
    );
    assert!(retention_candidates_for_replica_floors(&generations, 4, 5, &[99], &[1]).is_err());
    assert!(
        retention_candidates_for_replica_floors(&generations, 4, 1, &[2, 2], &[4]).is_err()
    );
}
