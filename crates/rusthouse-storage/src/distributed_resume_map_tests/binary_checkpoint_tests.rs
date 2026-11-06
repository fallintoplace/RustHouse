use super::*;

#[test]
fn compact_checkpoints_round_trip_at_boundary_sizes() {
    for count in [0, 1, 7, 8, 9, 16, 65_536] {
        let mut map = ResumeMap::new(count).unwrap();
        for index in (0..count).step_by(3) {
            map.mark_complete(index).unwrap();
        }
        let encoded = map.encode_binary();
        assert_eq!(encoded.len(), 4 + count.div_ceil(8));
        assert_eq!(ResumeMap::decode_binary(&encoded).unwrap(), map);
    }
}

#[test]
fn refuses_incorrect_lengths_counts_and_unused_tail_bits() {
    assert!(ResumeMap::decode_binary(&[1, 0, 0]).is_err());
    assert!(ResumeMap::decode_binary(&[1, 0, 0, 0]).is_err());
    assert!(ResumeMap::decode_binary(&[1, 0, 0, 0, 0x80]).is_err());
    assert!(ResumeMap::decode_binary(&[1, 0, 0, 0, 1, 0]).is_err());
    assert!(ResumeMap::decode_binary(&[1, 0, 1, 0]).is_err());
}
