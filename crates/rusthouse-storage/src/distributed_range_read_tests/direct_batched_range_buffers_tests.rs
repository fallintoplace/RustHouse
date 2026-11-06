use super::*;

#[test]
fn unordered_and_overlapping_ranges_fill_exact_caller_buffers() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-batch-range-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let contents = (0..=255u8).collect::<Vec<_>>();
    fs::write(&path, &contents).unwrap();
    let mut first = [0u8; 17];
    let mut second = [0u8; 7];
    let mut overlap = [0u8; 10];
    let mut empty = [];
    let mut ranges = [
        (100u64, first.as_mut_slice()),
        (3, second.as_mut_slice()),
        (105, overlap.as_mut_slice()),
        (256, empty.as_mut_slice()),
    ];
    read_object_ranges_into(&path, &mut ranges, 34).unwrap();
    assert_eq!(first.as_slice(), &contents[100..117]);
    assert_eq!(second.as_slice(), &contents[3..10]);
    assert_eq!(overlap.as_slice(), &contents[105..115]);

    let mut unchanged = [0xcc; 8];
    let mut ok = [0xaa; 2];
    {
        let mut invalid = [(0, ok.as_mut_slice()), (252, unchanged.as_mut_slice())];
        assert!(read_object_ranges_into(&path, &mut invalid, 10).is_err());
        assert!(read_object_ranges_into(&path, &mut invalid, 9).is_err());
    }
    assert_eq!(ok, [0xaa; 2]);
    assert_eq!(unchanged, [0xcc; 8]);
    fs::remove_file(path).unwrap();
}

#[test]
fn zero_ranges_keep_identity_validation_and_size_limits() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-batch-empty-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::write(&path, [1u8, 2, 3]).unwrap();
    let mut requests: [(u64, &mut [u8]); 0] = [];
    read_object_ranges_into(&path, &mut requests, 0).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(read_object_ranges_into(&path, &mut requests, 0).is_err());
}
