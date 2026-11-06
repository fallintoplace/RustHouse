use super::*;

#[test]
fn repeated_ranges_use_the_same_verification_and_reject_conflicts() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-duplicate-crc-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let bytes = (0..65_536usize)
        .map(|index| (index * 71 % 251) as u8)
        .collect::<Vec<_>>();
    fs::write(&path, &bytes).unwrap();
    let crc = crate::distributed_crc32c::crc32c(&bytes);
    let mut ranges = vec![(0u64, bytes.len() as u64, crc); 1024];
    verify_object_ranges_crc32c(&path, &ranges, 64 * 1024 * 1024).unwrap();

    ranges[511].2 ^= 1;
    assert_eq!(
        verify_object_ranges_crc32c(&path, &ranges, 64 * 1024 * 1024)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    ranges[511].2 = crc;
    assert!(verify_object_ranges_crc32c(&path, &ranges, 1024).is_err());

    let mut corrupted = bytes;
    corrupted[1234] ^= 1;
    fs::write(&path, &corrupted).unwrap();
    assert_eq!(
        verify_object_ranges_crc32c(&path, &ranges, 64 * 1024 * 1024)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    fs::remove_file(path).unwrap();
}
