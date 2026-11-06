use super::*;

#[test]
fn sparse_holes_cross_word_and_partial_byte_boundaries() {
    for len in [
        0usize, 1, 7, 8, 9, 63, 64, 65, 71, 72, 73, 511, 512, 513, 65_535, 65_536,
    ] {
        let mut map = ResumeMap::new(len).unwrap();
        map.mark_complete_range(0, len).unwrap();
        let holes = [
            0usize, 7, 8, 31, 32, 63, 64, 65, 71, 72, 127, 128, 511, 512, 65_534, 65_535,
        ];
        let expected = holes
            .iter()
            .copied()
            .filter(|&index| index < len)
            .collect::<Vec<_>>();
        for &index in &expected {
            map.invalidate_range(index, index + 1).unwrap();
        }
        assert_eq!(
            map.remaining_indices().collect::<Vec<_>>(),
            expected,
            "len={len}"
        );
        assert_eq!(map.remaining_count(), expected.len());
        assert_eq!(map.remaining(), expected);
        assert_eq!(map.remaining_indices().next(), expected.first().copied());
        assert_eq!(
            ResumeMap::decode_binary(&map.encode_binary())
                .unwrap()
                .remaining(),
            expected,
        );
    }
}

#[test]
fn partial_map_iterators_match_scalar_reference_for_dense_checkpoints() {
    for len in [63usize, 64, 65, 511, 512, 513, 8_191, 8_192, 65_536] {
        let mut map = ResumeMap::new(len).unwrap();
        for index in 0..len {
            if index % 7 != 0 && index % 101 != 0 {
                map.mark_complete(index).unwrap();
            }
        }
        let expected = (0..len)
            .filter(|&index| index % 7 == 0 || index % 101 == 0)
            .collect::<Vec<_>>();
        assert_eq!(map.remaining_indices().collect::<Vec<_>>(), expected);
    }
}
