use super::*;

#[test]
fn singleton_multi_range_reader_returns_exact_bytes() {
    let path =
        std::env::temp_dir().join(format!("rusthouse-single-range-{}", std::process::id()));
    fs::write(&path, b"abcdefgh").unwrap();
    assert_eq!(
        read_object_ranges(&path, &[(2, 4)], 4).unwrap(),
        vec![b"cdef".to_vec()]
    );
    assert_eq!(
        read_object_ranges(&path, &[(8, 0)], 0).unwrap(),
        vec![Vec::new()]
    );
    fs::remove_file(&path).unwrap();
}

#[test]
fn reads_exact_intervals_and_rejects_bad_ranges() {
    let path = std::env::temp_dir().join(format!("rusthouse-range-{}", std::process::id()));
    fs::write(&path, b"0123456789").unwrap();
    assert_eq!(read_object_range(&path, 3, 4, 8).unwrap(), b"3456");
    assert_eq!(read_object_range(&path, 10, 0, 8).unwrap(), b"");
    assert!(read_object_range(&path, 9, 2, 8).is_err());
    assert!(read_object_range(&path, u64::MAX, 2, 8).is_err());
    assert!(read_object_range(&path, 0, 9, 8).is_err());
    fs::remove_file(path).unwrap();
}
#[cfg(unix)]
#[test]
fn refuses_symlinked_range_sources() {
    use std::os::unix::fs::symlink;
    let dir = std::env::temp_dir().join(format!("rusthouse-range-link-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let original = dir.join("object");
    let link = dir.join("link");
    fs::write(&original, b"abc").unwrap();
    symlink(&original, &link).unwrap();
    assert!(read_object_range(&link, 0, 1, 2).is_err());
    fs::remove_dir_all(dir).unwrap();
}
