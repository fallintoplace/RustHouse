use super::*;

#[test]
fn single_checked_reads_preserve_bounds_empty_ranges_and_crc_contract() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-checked-single-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let bytes = (0u32..256).map(|n| (n % 251) as u8).collect::<Vec<_>>();
    fs::write(&path, &bytes).unwrap();
    let checksum = crate::distributed_crc32c::crc32c;
    let correct = (17, 64, checksum(&bytes[17..81]));
    assert_eq!(
        read_object_ranges_checked(&path, &[correct], 64).unwrap(),
        vec![bytes[17..81].to_vec()]
    );
    assert_eq!(
        read_object_ranges_checked(&path, &[(bytes.len() as u64, 0, checksum(b""))], 0)
            .unwrap(),
        vec![Vec::<u8>::new()]
    );
    let bad = (17, 64, correct.2 ^ 1);
    assert_eq!(
        read_object_ranges_checked(&path, &[bad], 64)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert!(read_object_ranges_checked(&path, &[correct], 63).is_err());
    assert!(read_object_ranges_checked(&path, &[(250, 7, 0)], 64).is_err());
    fs::remove_file(path).unwrap();
}
