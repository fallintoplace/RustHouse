use super::*;

#[test]
fn dense_sorted_pins_keep_identical_candidates_for_reordered_inputs() {
    let generations = (1..=16_384u64).collect::<Vec<_>>();
    let mut reversed = generations.clone();
    reversed.reverse();
    let pins = generations.iter().copied().step_by(3).collect::<Vec<_>>();
    let mut unordered_pins = pins.clone();
    unordered_pins.reverse();
    let expected =
        retention_candidates_with_pins(&reversed, 16_384, 2, &unordered_pins).unwrap();
    assert_eq!(
        retention_candidates_with_pins(&generations, 16_384, 2, &pins).unwrap(),
        expected
    );
    assert!(
        expected
            .iter()
            .all(|item| pins.binary_search(item).is_err())
    );
    assert!(!expected.contains(&16_384));
    assert!(!expected.contains(&16_383));
}

#[test]
fn unknown_sorted_pin_between_valid_generations_fails_closed() {
    let known = [2, 4, 6, 8, 10, 12];
    assert!(retention_candidates_with_pins(&known, 12, 1, &[2, 5, 8, 10, 12]).is_err());
    assert!(retention_candidates_with_pins(&known, 12, 1, &[2, 4, 6, 8, 14]).is_err());
    assert!(retention_candidates_with_pins(&known, 12, 1, &[2, 4, 4, 8, 10]).is_err());
}
