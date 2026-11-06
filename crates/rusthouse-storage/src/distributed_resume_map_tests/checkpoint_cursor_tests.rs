use super::*;

#[test]
fn continuation_iterators_match_full_bitmap_scan_at_every_boundary() {
    for chunks in [0, 1, 7, 8, 9, 63, 64, 65, 511, 512, 513, 65_535, 65_536] {
        let mut map = ResumeMap::new(chunks).unwrap();
        for index in 0..chunks {
            if index % 3 != 1 && index % 11 != 4 {
                map.mark_complete(index).unwrap();
            }
        }
        let full = map.remaining();
        for start in [
            0,
            1,
            7,
            8,
            63,
            64,
            chunks / 2,
            chunks,
            chunks + 1,
            usize::MAX,
        ] {
            let expected = full
                .iter()
                .copied()
                .filter(|&index| index >= start)
                .collect::<Vec<_>>();
            assert_eq!(
                map.remaining_indices_from(start).collect::<Vec<_>>(),
                expected
            );
        }
    }
}
