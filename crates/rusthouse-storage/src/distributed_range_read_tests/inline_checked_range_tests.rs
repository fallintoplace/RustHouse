use super::*;

#[test]
fn inline_and_heap_range_paths_preserve_order_and_individual_checksums() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-checked-ranges-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("object.bin");
    let bytes = (0..4096usize).map(|i| (i % 251) as u8).collect::<Vec<_>>();
    std::fs::write(&path, &bytes).unwrap();
    for count in [0usize, 1, 2, 8, 9, 16] {
        let ranges = (0..count)
            .map(|index| {
                let offset = ((count - index) * 7) as u64;
                let len = 23usize;
                let crc = crate::distributed_crc32c::crc32c(
                    &bytes[offset as usize..offset as usize + len],
                );
                (offset, len, crc)
            })
            .collect::<Vec<_>>();
        let actual = read_object_ranges_checked(&path, &ranges, 4096).unwrap();
        assert_eq!(actual.len(), count);
        for (result, &(offset, len, _)) in actual.iter().zip(&ranges) {
            assert_eq!(result, &bytes[offset as usize..offset as usize + len]);
        }
        if !ranges.is_empty() {
            let mut corrupt = ranges;
            corrupt[0].2 ^= 1;
            assert!(read_object_ranges_checked(&path, &corrupt, 4096).is_err());
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
