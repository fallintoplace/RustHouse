use super::*;

#[test]
fn sorted_inventory_avoids_copying_and_unsorted_inventory_still_sorts() {
    let ordered = (1..=16_384u64).collect::<Vec<_>>();
    let unordered = ordered.iter().rev().copied().collect::<Vec<_>>();
    assert!(matches!(
        validated_generations(&ordered, 42).unwrap(),
        Cow::Borrowed(_)
    ));
    assert!(matches!(
        validated_generations(&unordered, 42).unwrap(),
        Cow::Owned(_)
    ));

    for active in [1, 1000, 16_384] {
        for keep in [1, 2, 1024, 16_384, usize::MAX] {
            assert_eq!(
                retention_candidates(&ordered, active, keep).unwrap(),
                retention_candidates(&unordered, active, keep).unwrap(),
            );
        }
    }
    let pins = [1, 15, 8192, 16384];
    assert_eq!(
        retention_candidates_with_pins(&ordered, 16_384, 1, &pins).unwrap(),
        retention_candidates_with_pins(&unordered, 16_384, 1, &pins).unwrap(),
    );
}

#[test]
fn borrowed_path_still_rejects_invalid_generation_inventories() {
    assert!(validated_generations(&[], 1).is_err());
    assert!(validated_generations(&[0, 1, 2], 2).is_err());
    assert!(validated_generations(&[1, 2, 3], 4).is_err());
    assert!(validated_generations(&[2, 2, 3], 3).is_err());
}
