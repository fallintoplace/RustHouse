use super::*;

#[test]
fn reads_prefix_and_nonzero_offsets_in_both_range_apis() {
    let path =
        std::env::temp_dir().join(format!("rusthouse-range-zero-seek-{}", std::process::id()));
    fs::write(&path, b"abcdefghijkl").unwrap();
    assert_eq!(read_object_range(&path, 0, 4, 12).unwrap(), b"abcd");
    assert_eq!(read_object_range(&path, 4, 4, 12).unwrap(), b"efgh");
    assert_eq!(
        read_object_ranges(&path, &[(0, 4)], 4).unwrap(),
        vec![b"abcd".to_vec()]
    );
    assert_eq!(
        read_object_ranges(&path, &[(8, 2), (0, 3)], 5).unwrap(),
        vec![b"ij".to_vec(), b"abc".to_vec()]
    );
    assert_eq!(
        read_object_ranges(&path, &[(0, 6), (3, 6)], 12).unwrap(),
        vec![b"abcdef".to_vec(), b"defghi".to_vec()]
    );
    fs::remove_file(path).unwrap();
}
