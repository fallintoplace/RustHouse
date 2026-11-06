use super::*;

#[test]
fn appended_blocks_preserve_standalone_wire_format() {
    let repeated = vec![0x41; 64 * 1024];
    let irregular = (0..(16 * 1024))
        .map(|n| (n % 251) as u8)
        .collect::<Vec<_>>();
    for codec in [PartCompression::Lz4, PartCompression::Zstd] {
        let mut output = vec![7u8, 8u8, 9u8];
        for input in [&repeated[..], &irregular[..], &[1u8, 2, 3][..]] {
            let before = output.len();
            let encoded = block(input, codec).unwrap();
            append_block(&mut output, input, codec).unwrap();
            assert_eq!(&output[before..], encoded.as_slice());
        }
        assert_eq!(&output[..3], &[7, 8, 9]);
    }
}
