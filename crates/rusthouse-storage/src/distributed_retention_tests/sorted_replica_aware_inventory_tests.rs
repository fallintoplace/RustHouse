use super::*;

#[test]
fn borrowed_and_reversed_inventories_produce_identical_retention_plans() {
    let sorted = (1..=10_000u64)
        .map(|generation| (generation, generation % 7 + 1))
        .collect::<Vec<_>>();
    assert!(matches!(
        ordered_generation_sizes(&sorted),
        Cow::Borrowed(_)
    ));
    let mut reversed = sorted.clone();
    reversed.reverse();
    assert!(matches!(ordered_generation_sizes(&reversed), Cow::Owned(_)));

    for pins in [
        &[][..],
        &[77, 500, 9000][..],
        &[77, 500, 900, 2000, 9000][..],
    ] {
        let expected = retention_candidates_with_replica_floors_and_budget(
            &sorted,
            10_000,
            3,
            pins,
            &[9_000, 9_700],
            3_000,
        )
        .unwrap();
        let actual = retention_candidates_with_replica_floors_and_budget(
            &reversed,
            10_000,
            3,
            pins,
            &[9_000, 9_700],
            3_000,
        )
        .unwrap();
        assert_eq!(actual, expected);
        assert!(actual.0.iter().all(|&generation| generation < 9_000));
        assert!(actual.0.iter().all(|generation| !pins.contains(generation)));
        assert!(actual.1 <= 3_000);
    }

    assert!(
        retention_candidates_with_replica_floors_and_budget(
            &sorted,
            10_000,
            3,
            &[77, 77],
            &[9000],
            0
        )
        .is_err()
    );
}
