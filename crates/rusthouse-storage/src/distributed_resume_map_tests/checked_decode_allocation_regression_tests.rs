use super::*;

#[test]
fn checked_decoder_keeps_all_boundary_layouts_and_tail_checks() {
    for chunks in [0usize, 1, 7, 8, 9, 63, 64, 65, 65_535, 65_536] {
        let mut map = ResumeMap::new(chunks).unwrap();
        for index in (0..chunks).step_by(3) {
            map.mark_complete(index).unwrap();
        }
        let wire = map.encode_binary_checked();
        assert_eq!(ResumeMap::decode_binary_checked(&wire).unwrap(), map);
        let mut corrupt = wire;
        if chunks % 8 != 0 {
            let last_bitmap = 8 + chunks / 8;
            corrupt[last_bitmap] |= 0x80;
            let crc = crate::distributed_crc32c::crc32c(&corrupt[..corrupt.len() - 4]);
            let end = corrupt.len();
            corrupt[end - 4..].copy_from_slice(&crc.to_le_bytes());
            assert!(ResumeMap::decode_binary_checked(&corrupt).is_err());
        }
    }
}
