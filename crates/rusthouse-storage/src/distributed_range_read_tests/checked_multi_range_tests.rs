use super::*;

#[test]
fn verifies_overlapping_chunk_checksums_and_rejects_corruption() {
    let path =
        std::env::temp_dir().join(format!("rusthouse-checked-range-{}", std::process::id()));
    let bytes = b"the quick brown fox jumped over a lazy dog";
    fs::write(&path, bytes).unwrap();
    let checksum = crate::distributed_crc32c::crc32c;
    let ranges = vec![
        (0, 9, checksum(&bytes[0..9])),
        (4, 16, checksum(&bytes[4..20])),
        (bytes.len() as u64, 0, checksum(b"")),
        (2, 4, checksum(&bytes[2..6])),
    ];
    let result = read_object_ranges_checked(&path, &ranges, 32).unwrap();
    assert_eq!(
        result,
        vec![
            bytes[0..9].to_vec(),
            bytes[4..20].to_vec(),
            Vec::new(),
            bytes[2..6].to_vec(),
        ]
    );
    let mut corrupted = ranges.clone();
    corrupted[1].2 ^= 1;
    assert_eq!(
        read_object_ranges_checked(&path, &corrupted, 32)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert!(read_object_ranges_checked(&path, &ranges, 8).is_err());
    fs::remove_file(path).unwrap();
}
