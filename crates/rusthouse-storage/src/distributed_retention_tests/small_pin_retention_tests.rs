use super::*;

#[test]
fn low_cardinality_pins_preserve_large_path_semantics() {
    let generations = (1..=130u64).collect::<Vec<_>>();
    for pins in [
        vec![],
        vec![1],
        vec![1, 3],
        vec![1, 3, 5, 7],
        vec![1, 3, 5, 7, 9],
    ] {
        let small = retention_candidates_with_pins(&generations, 130, 2, &pins).unwrap();
        assert_eq!(
            small,
            (1..=128u64)
                .filter(|id| !pins.contains(id))
                .collect::<Vec<_>>()
        );
    }
    assert!(retention_candidates_with_pins(&generations, 130, 2, &[1, 1]).is_err());
    assert!(retention_candidates_with_pins(&generations, 130, 2, &[131]).is_err());
}
