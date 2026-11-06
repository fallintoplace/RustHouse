use super::*;

#[test]
fn versioned_checkpoints_round_trip_at_every_bitmap_boundary() {
    for chunks in [0, 1, 7, 8, 9, 63, 64, 65, 511, 512, 513, 65_535, 65_536] {
        let mut map = ResumeMap::new(chunks).unwrap();
        for index in (0..chunks).step_by(3) {
            map.mark_complete(index).unwrap();
        }
        let encoded = map.encode_binary_checked();
        assert_eq!(&encoded[..4], b"RSM1");
        assert_eq!(encoded.len(), 12 + chunks.div_ceil(8));
        assert_eq!(ResumeMap::decode_binary_checked(&encoded).unwrap(), map);
        assert_eq!(ResumeMap::decode_binary(&map.encode_binary()).unwrap(), map);
    }
}

#[test]
fn every_single_bit_corruption_rejects_before_resuming_work() {
    for chunks in [1, 8, 9, 1024] {
        let mut map = ResumeMap::new(chunks).unwrap();
        for index in (0..chunks).step_by(2) {
            map.mark_complete(index).unwrap();
        }
        let encoded = map.encode_binary_checked();
        for position in 0..encoded.len() {
            for bit in 0..8 {
                let mut corrupted = encoded.clone();
                corrupted[position] ^= 1 << bit;
                assert!(
                    ResumeMap::decode_binary_checked(&corrupted).is_err(),
                    "chunks={chunks}, position={position}, bit={bit}"
                );
            }
        }
    }
}

#[test]
fn length_magic_and_unused_tail_bits_are_strictly_validated() {
    let map = ResumeMap::new(9).unwrap();
    let encoded = map.encode_binary_checked();
    assert!(ResumeMap::decode_binary_checked(&encoded[..encoded.len() - 1]).is_err());
    let mut extra = encoded.clone();
    extra.push(0);
    assert!(ResumeMap::decode_binary_checked(&extra).is_err());
    let mut wrong = encoded.clone();
    wrong[..4].copy_from_slice(b"RSM2");
    assert!(ResumeMap::decode_binary_checked(&wrong).is_err());
    let mut bad_tail = encoded.clone();
    bad_tail[9] = 0x80;
    let crc = crate::distributed_crc32c::crc32c(&bad_tail[..bad_tail.len() - 4]);
    let length = bad_tail.len();
    bad_tail[length - 4..].copy_from_slice(&crc.to_le_bytes());
    assert!(ResumeMap::decode_binary_checked(&bad_tail).is_err());
    assert!(ResumeMap::decode_binary_checked(b"").is_err());
}
