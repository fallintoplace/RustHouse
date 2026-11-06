use super::*;

#[test]
fn overlapping_and_adjacent_ranges_preserve_request_order() {
    let path =
        std::env::temp_dir().join(format!("rusthouse-overlap-ranges-{}", std::process::id()));
    fs::write(&path, b"abcdefghijklmnop").unwrap();
    let selections = [(8, 4), (0, 4), (2, 5), (7, 1), (8, 0), (14, 2)];
    let ranges = read_object_ranges(&path, &selections, 17).unwrap();
    assert_eq!(
        ranges,
        vec![
            b"ijkl".to_vec(),
            b"abcd".to_vec(),
            b"cdefg".to_vec(),
            b"h".to_vec(),
            Vec::new(),
            b"op".to_vec(),
        ]
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn disjoint_large_ranges_return_exact_data_in_request_order() {
    let path =
        std::env::temp_dir().join(format!("rusthouse-disjoint-ranges-{}", std::process::id()));
    let bytes = (0..256 * 1024)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    fs::write(&path, &bytes).unwrap();
    let ranges = [
        (128 * 1024, 64 * 1024),
        (0, 32 * 1024),
        (224 * 1024, 16 * 1024),
    ];
    let result = read_object_ranges(&path, &ranges, 112 * 1024).unwrap();
    for (index, &(start, len)) in ranges.iter().enumerate() {
        assert_eq!(&result[index], &bytes[start as usize..start as usize + len]);
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn reads_multiple_intervals_in_request_order() {
    let path =
        std::env::temp_dir().join(format!("rusthouse-multi-range-{}", std::process::id()));
    fs::write(&path, b"0123456789").unwrap();
    assert_eq!(
        read_object_ranges(&path, &[(8, 2), (0, 3), (5, 0)], 5).unwrap(),
        vec![b"89".to_vec(), b"012".to_vec(), Vec::new()]
    );
    assert!(read_object_ranges(&path, &[(0, 3), (8, 2)], 4).is_err());
    assert!(read_object_ranges(&path, &[(9, 2)], 8).is_err());
    assert!(read_object_ranges(&path, &[(u64::MAX, 2)], 8).is_err());
    fs::remove_file(path).unwrap();
}
