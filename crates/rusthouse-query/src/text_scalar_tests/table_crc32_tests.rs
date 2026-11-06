use super::*;

fn reference(input: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in input {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

#[test]
fn table_matches_bitwise_reference_for_binary_and_unaligned_buffers() {
    let mut source = vec![0u8; 8192];
    let mut state = 0x9e37_79b9u32;
    for byte in &mut source {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        *byte = state as u8;
    }
    for length in [
        0, 1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 63, 127, 255, 1024, 8192,
    ] {
        assert_eq!(crc32_ieee(&source[..length]), reference(&source[..length]));
    }
    for offset in 0..16 {
        assert_eq!(
            crc32_ieee(&source[offset..offset + 4096]),
            reference(&source[offset..offset + 4096])
        );
    }
    assert_eq!(crc32_ieee(b"123456789"), 0xcbf4_3926);
}
