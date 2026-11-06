use super::*;

#[test]
fn paged_differences_match_bitwise_oracle_at_word_boundaries() {
    for chunks in [
        0, 1, 7, 8, 9, 63, 64, 65, 127, 128, 129, 1023, 65_535, 65_536,
    ] {
        let mut left = ResumeMap::new(chunks).unwrap();
        let mut right = ResumeMap::new(chunks).unwrap();
        for index in 0..chunks {
            if index % 5 < 3 {
                left.mark_complete(index).unwrap();
            }
            if index % 7 < 3 {
                right.mark_complete(index).unwrap();
            }
        }
        for cursor in [0, 1, 7, 8, 63, 64, 65, chunks / 2, chunks] {
            if cursor > chunks {
                continue;
            }
            let expected = (cursor..chunks)
                .filter(|&index| (index % 5 < 3) != (index % 7 < 3))
                .collect::<Vec<_>>();
            for limit in [0, 1, 3, 64, 65_536] {
                let actual = left.differing_indices_from(&right, cursor, limit).unwrap();
                assert_eq!(
                    actual,
                    expected[..expected.len().min(limit)],
                    "chunks={chunks}, cursor={cursor}, limit={limit}"
                );
            }
        }
    }
}

#[test]
fn refuses_mismatched_layouts_and_oversized_pages() {
    let a = ResumeMap::new(8).unwrap();
    let b = ResumeMap::new(9).unwrap();
    assert!(a.differing_indices_from(&b, 0, 1).is_err());
    assert!(a.differing_indices_from(&a, 9, 1).is_err());
    assert!(a.differing_indices_from(&a, 0, 65_537).is_err());
}
