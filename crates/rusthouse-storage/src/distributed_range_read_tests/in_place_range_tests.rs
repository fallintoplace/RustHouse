use super::*;

#[test]
fn existing_buffers_match_owned_exact_reads_at_unaligned_offsets() {
    let directory = std::env::temp_dir().join(format!(
        "rusthouse-inplace-range-{}-{}",
        std::process::id(),
        line!()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("object.bin");
    let source = (0..16384u32)
        .map(|value| (value % 251) as u8)
        .collect::<Vec<_>>();
    fs::write(&path, &source).unwrap();
    for &(offset, length) in &[(0, 0), (0, 1), (3, 7), (4095, 4096), (16384, 0)] {
        let mut destination = vec![0xcc; length];
        read_object_range_into(&path, offset, &mut destination, length).unwrap();
        assert_eq!(
            destination,
            read_object_range(&path, offset, length, length).unwrap()
        );
    }
    let mut untouched = [0xabu8; 16];
    assert!(read_object_range_into(&path, 16380, &mut untouched, 16).is_err());
    assert_eq!(untouched, [0xab; 16]);
    assert!(read_object_range_into(&path, 0, &mut untouched, 15).is_err());
    assert_eq!(untouched, [0xab; 16]);
    fs::remove_dir_all(directory).unwrap();
}
