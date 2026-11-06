use super::*;

#[test]
fn four_reader_pins_filter_large_inventories_without_a_hash_table() {
    let generations = (1..=16_384u64).collect::<Vec<_>>();
    let pins = [1, 17, 8_192, 16_380];
    let candidates = retention_candidates_with_pins(&generations, 16_384, 2, &pins).unwrap();
    assert_eq!(candidates.len(), 16_382 - pins.len());
    for pin in pins {
        assert!(!candidates.contains(&pin));
    }
    assert_eq!(candidates.first(), Some(&2));
    assert_eq!(candidates.last(), Some(&16_382));
    assert!(
        retention_candidates_with_pins(&generations, 16_384, 2, &[1, 17, 17, 8_192],).is_err()
    );
    assert!(
        retention_candidates_with_pins(&generations, 16_384, 2, &[1, 17, 8_192, 16_385],)
            .is_err()
    );
}
