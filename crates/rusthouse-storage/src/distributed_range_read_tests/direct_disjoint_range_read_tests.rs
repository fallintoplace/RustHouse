use super::*;

#[test]
fn ordered_sparse_ranges_and_zero_lengths_match_exact_file_slices() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-range-disjoint-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("part.bin");
    let data = (0..8192usize)
        .map(|n| (n * 31 % 256) as u8)
        .collect::<Vec<_>>();
    fs::write(&path, &data).unwrap();
    for ranges in [
        vec![(0, 32), (64, 32), (200, 800), (4000, 100), (8100, 92)],
        vec![(0, 0), (1, 17), (1, 0), (64, 200), (8192, 0)],
        vec![(50, 10), (60, 10), (70, 10)],
        vec![(400, 50), (425, 100), (400, 50), (10, 10)],
    ] {
        let total = ranges.iter().map(|(_, length)| *length).sum::<usize>();
        let actual = read_object_ranges(&path, &ranges, total).unwrap();
        let expected = ranges
            .iter()
            .map(|&(offset, length)| data[offset as usize..offset as usize + length].to_vec())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "ranges={ranges:?}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sorted_disjoint_reads_still_check_total_budget_and_bounds() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-range-bounds-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("data");
    fs::write(&path, vec![7u8; 256]).unwrap();
    assert!(read_object_ranges(&path, &[(0, 4), (20, 4)], 7).is_err());
    assert!(read_object_ranges(&path, &[(0, 4), (250, 7)], 100).is_err());
    fs::remove_dir_all(root).unwrap();
}
