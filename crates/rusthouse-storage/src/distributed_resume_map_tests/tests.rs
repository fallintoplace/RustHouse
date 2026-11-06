use super::*;

#[test]
fn checkpoints_and_resumes_missing_chunks() {
    let mut map = ResumeMap::new(4).unwrap();
    map.mark_complete(0).unwrap();
    map.mark_complete(2).unwrap();
    let recovered = ResumeMap::decode(&map.encode()).unwrap();
    assert_eq!(recovered.remaining(), vec![1, 3]);
    assert!(!recovered.complete());
}

#[test]
fn reports_chunk_progress_without_allocating_missing_indexes() {
    for chunks in [0usize, 1, 7, 8, 9, 17, 65_536] {
        let mut map = ResumeMap::new(chunks).unwrap();
        assert_eq!(map.completed_count(), 0);
        assert_eq!(map.remaining_count(), chunks);
        for index in (0..chunks).step_by(3) {
            map.mark_complete(index).unwrap();
            map.mark_complete(index).unwrap();
        }
        let complete = chunks.div_ceil(3);
        assert_eq!(map.completed_count(), complete);
        assert_eq!(map.remaining_count(), chunks - complete);
        assert_eq!(map.remaining().len(), map.remaining_count());
    }
}

#[test]
fn range_completion_matches_individual_marks_at_byte_boundaries() {
    for len in [0usize, 1, 7, 8, 9, 16, 17, 31] {
        for start in 0..=len {
            for end in start..=len {
                let mut bulk = ResumeMap::new(len).unwrap();
                let mut separate = ResumeMap::new(len).unwrap();
                bulk.mark_complete_range(start, end).unwrap();
                for index in start..end {
                    separate.mark_complete(index).unwrap();
                }
                assert_eq!(bulk, separate, "{len} {start} {end}");
            }
        }
    }
    let mut map = ResumeMap::new(17).unwrap();
    assert!(map.mark_complete_range(1, 18).is_err());
    assert!(map.mark_complete_range(10, 9).is_err());
    assert_eq!(map.remaining(), (0..17).collect::<Vec<_>>());
    map.mark_complete_range(0, 17).unwrap();
    assert!(map.complete());
}

#[test]
fn bitmap_roundtrips_across_byte_boundaries() {
    for chunks in [0usize, 1, 7, 8, 9, 15, 16, 17, 65_536] {
        let mut progress = ResumeMap::new(chunks).unwrap();
        assert_eq!(progress.len(), chunks);
        assert_eq!(progress.is_empty(), chunks == 0);
        for index in (0..chunks).step_by(3) {
            progress.mark_complete(index).unwrap();
        }
        let restored = ResumeMap::decode(&progress.encode()).unwrap();
        assert_eq!(restored, progress);
        assert_eq!(restored.remaining().len(), chunks - chunks.div_ceil(3));
        assert_eq!(restored.complete(), chunks == 0 || chunks == 1);
        for index in 0..chunks {
            progress.mark_complete(index).unwrap();
        }
        assert!(progress.complete());
        assert_eq!(progress.remaining(), Vec::<usize>::new());
    }
}

#[test]
fn rejects_invalid_progress_and_bounds() {
    assert!(ResumeMap::new(65_537).is_err());
    assert!(ResumeMap::decode("10x01").is_err());
    assert!(ResumeMap::new(1).unwrap().mark_complete(2).is_err());
}
#[test]
fn completion_checks_only_used_bits_in_partial_tail() {
    let mut progress = ResumeMap::new(9).unwrap();
    for index in 0..8 {
        progress.mark_complete(index).unwrap();
    }
    assert!(!progress.complete());
    progress.mark_complete(8).unwrap();
    assert!(progress.complete());
    assert!(ResumeMap::new(0).unwrap().complete());
}

#[test]
fn remaining_ignores_unused_tail_bits() {
    for len in [1, 2, 7, 8, 9, 10, 15, 16, 17] {
        let mut progress = ResumeMap::new(len).unwrap();
        for index in (0..len).step_by(2) {
            progress.mark_complete(index).unwrap();
        }
        let expected = (0..len).filter(|index| index % 2 == 1).collect::<Vec<_>>();
        assert_eq!(progress.remaining(), expected);
    }
}

#[test]
fn sparse_and_dense_ascii_maps_match_the_bitmap_reference() {
    for count in [0, 1, 7, 8, 9, 513, 65_536] {
        for step in [1, 2, 7, 31] {
            let mut map = ResumeMap::new(count).unwrap();
            for index in (0..count).step_by(step) {
                map.mark_complete(index).unwrap();
            }
            let expected = (0..count)
                .map(|index| {
                    let (byte, mask) = ResumeMap::bit_position(index);
                    if map.completed[byte] & mask == 0 {
                        '0'
                    } else {
                        '1'
                    }
                })
                .collect::<String>();
            assert_eq!(map.encode(), expected);
        }
    }
}

#[test]
fn encoder_keeps_partial_byte_flags_in_index_order() {
    let mut map = ResumeMap::new(11).unwrap();
    for index in [0, 2, 7, 8, 10] {
        map.mark_complete(index).unwrap();
    }
    assert_eq!(map.encode(), "10100001101");
}

#[test]
fn decoding_large_bitmaps_keeps_exact_bit_positions() {
    let input = "10100111".repeat(8192);
    let decoded = ResumeMap::decode(&input).unwrap();
    assert_eq!(decoded.len(), 65_536);
    assert_eq!(decoded.encode(), input);
    assert_eq!(decoded.remaining().len(), 3 * 8192);
}

#[test]
fn partial_bitmap_reserves_exact_remaining_slots() {
    let mut progress = ResumeMap::new(65_536).unwrap();
    for index in (0..65_536).step_by(2) {
        progress.mark_complete(index).unwrap();
    }
    let remaining = progress.remaining();
    assert_eq!(remaining.len(), 32_768);
    assert_eq!(remaining.capacity(), remaining.len());
    assert_eq!(remaining[0], 1);
    assert_eq!(*remaining.last().unwrap(), 65_535);
}
