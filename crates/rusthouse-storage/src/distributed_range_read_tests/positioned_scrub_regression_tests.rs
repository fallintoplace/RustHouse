use super::*;

#[test]
fn unaligned_out_of_order_and_overlapping_scrubs_match_reference_crc() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-positioned-scrub-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_file(&root);
    let bytes = (0..(256 * 1024 + 17))
        .map(|index| ((index * 67 + index / 13) % 251) as u8)
        .collect::<Vec<_>>();
    fs::write(&root, &bytes).unwrap();
    let ranges = [
        (120_011usize, 80_003usize),
        (3, 190_000),
        (bytes.len(), 0),
        (70_017, 65_537),
        (1, 1),
        (0, bytes.len()),
    ];
    let checksummed = ranges.map(|(start, length)| {
        (
            start as u64,
            length as u64,
            crate::distributed_crc32c::crc32c(&bytes[start..start + length]),
        )
    });
    let budget = checksummed.iter().map(|(_, length, _)| *length).sum();
    verify_object_ranges_crc32c(&root, &checksummed, budget).unwrap();
    let mut invalid = checksummed;
    invalid[3].2 ^= 0x4000;
    assert_eq!(
        verify_object_ranges_crc32c(&root, &invalid, budget)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    fs::remove_file(root).unwrap();
}
