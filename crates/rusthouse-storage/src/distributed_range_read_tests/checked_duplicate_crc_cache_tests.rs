use super::*;

#[test]
fn repeated_ranges_preserve_output_and_detect_conflicting_checksums() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-checked-duplicate-{}-{}",
        std::process::id(),
        line!()
    ));
    let bytes = (0..131_072usize)
        .map(|index| ((index * 37 + 11) % 251) as u8)
        .collect::<Vec<_>>();
    std::fs::write(&path, &bytes).unwrap();

    let spans = [(0u64, 4096usize), (21, 8000), (65_536, 32_768)];
    let requests = (0..64)
        .map(|index| {
            let (offset, length) = spans[index % spans.len()];
            let checksum = crate::distributed_crc32c::crc32c(
                &bytes[offset as usize..offset as usize + length],
            );
            (offset, length, checksum)
        })
        .collect::<Vec<_>>();
    let output = read_object_ranges_checked(&path, &requests, 2 * 1024 * 1024).unwrap();
    for (actual, &(offset, length, _)) in output.iter().zip(&requests) {
        assert_eq!(actual, &bytes[offset as usize..offset as usize + length]);
    }

    let mut corrupted = requests.clone();
    corrupted[47].2 ^= 1;
    assert!(matches!(
        read_object_ranges_checked(&path, &corrupted, 2 * 1024 * 1024),
        Err(ref error) if error.kind() == io::ErrorKind::InvalidData
    ));
    std::fs::remove_file(path).unwrap();
}
