use super::*;

#[test]
fn validates_large_sparse_overlapping_ranges_without_retaining_bytes() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-stream-check-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let bytes = (0..(512 * 1024))
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    fs::write(&path, &bytes).unwrap();
    let checksum = crate::distributed_crc32c::crc32c;
    let ranges = [
        (0, bytes.len() as u64, checksum(&bytes)),
        (1024, 65_537, checksum(&bytes[1024..1024 + 65_537])),
        (bytes.len() as u64, 0, checksum(b"")),
        (0, 1, checksum(&bytes[..1])),
    ];
    verify_object_ranges_crc32c(&path, &ranges, 700_000).unwrap();
    let mut bad = ranges;
    bad[1].2 ^= 1;
    assert_eq!(
        verify_object_ranges_crc32c(&path, &bad, 700_000)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData,
    );
    assert!(verify_object_ranges_crc32c(&path, &ranges, 1024).is_err());
    assert!(verify_object_ranges_crc32c(&path, &[(u64::MAX, 2, 0)], u64::MAX).is_err());
    assert!(verify_object_ranges_crc32c(&path, &vec![(0, 0, 0); 1025], u64::MAX).is_err());
    fs::remove_file(path).unwrap();
}

#[test]
fn streamed_crc_agrees_with_checked_range_reader_for_small_ranges() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-stream-compare-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let bytes = (0..16_384)
        .map(|i| (i * 13 % 251) as u8)
        .collect::<Vec<_>>();
    fs::write(&path, &bytes).unwrap();
    let mut ranges = Vec::new();
    for index in 0..48usize {
        let start = index * 241;
        let len = index % 193;
        let expected = crate::distributed_crc32c::crc32c(&bytes[start..start + len]);
        ranges.push((start as u64, len as u64, expected));
    }
    verify_object_ranges_crc32c(&path, &ranges, 48 * 193).unwrap();
    let read_ranges = ranges
        .iter()
        .map(|&(offset, len, checksum)| (offset, len as usize, checksum))
        .collect::<Vec<_>>();
    assert!(read_object_ranges_checked(&path, &read_ranges, 48 * 193).is_ok());
    fs::remove_file(path).unwrap();
}
