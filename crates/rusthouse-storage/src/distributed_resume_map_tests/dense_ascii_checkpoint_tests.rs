use super::*;

#[test]
fn every_packed_byte_expands_into_exact_bit_order() {
    for byte in 0u8..=u8::MAX {
        let ascii = ASCII_BITMAP_BYTES[usize::from(byte)];
        for (index, &character) in ascii.iter().enumerate() {
            assert_eq!(
                character,
                if byte & (1 << index) != 0 { b'1' } else { b'0' }
            );
        }
    }
}

#[test]
fn dense_maps_roundtrip_full_words_and_partial_tails() {
    for count in [256, 257, 263, 511, 512, 513, 8191, 8192, 65_535, 65_536] {
        let mut map = ResumeMap::new(count).unwrap();
        for index in 0..count {
            if index % 11 != 3 {
                map.mark_complete(index).unwrap();
            }
        }
        let encoded = map.encode();
        assert_eq!(encoded.len(), count);
        for (index, &character) in encoded.as_bytes().iter().enumerate() {
            assert_eq!(character, if index % 11 != 3 { b'1' } else { b'0' });
        }
        assert_eq!(ResumeMap::decode(&encoded).unwrap(), map);
    }
}

#[test]
fn sparse_checkpoints_still_encode_unset_flags() {
    let mut map = ResumeMap::new(8193).unwrap();
    for index in [0, 7, 64, 8192] {
        map.mark_complete(index).unwrap();
    }
    let encoded = map.encode();
    assert_eq!(encoded.bytes().filter(|&byte| byte == b'1').count(), 4);
    assert_eq!(ResumeMap::decode(&encoded).unwrap(), map);
}
