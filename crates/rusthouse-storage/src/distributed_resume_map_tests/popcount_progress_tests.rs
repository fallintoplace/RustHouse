use super::*;

#[test]
fn packed_popcounts_match_each_bit_across_partial_tail_bytes() {
    for chunks in [0, 1, 2, 7, 8, 9, 15, 16, 17, 63, 64, 65, 65_536] {
        let mut progress = ResumeMap::new(chunks).unwrap();
        for index in 0..chunks {
            if index % 3 != 1 {
                progress.mark_complete(index).unwrap();
            }
        }
        let expected = (0..chunks)
            .filter(|&index| {
                let (byte, mask) = ResumeMap::bit_position(index);
                progress.completed[byte] & mask != 0
            })
            .count();
        assert_eq!(progress.completed_count(), expected);
    }
}
