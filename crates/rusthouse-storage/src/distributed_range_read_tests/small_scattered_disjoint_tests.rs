use super::*;

#[test]
fn keeps_scattered_disjoint_requests_on_the_direct_read_path() {
    assert!(small_scattered_disjoint(&[
        (15, 3),
        (0, 2),
        (8, 4),
        (21, 0)
    ]));
    assert!(!small_scattered_disjoint(&[(15, 3), (17, 3)]));
    assert!(!small_scattered_disjoint(&[(10, 2), (12, 3)]));
    assert!(!small_scattered_disjoint(&[(0, 1); 9]));
}

#[test]
fn reads_out_of_order_small_ranges_in_original_order() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-scattered-range-{}-{}",
        std::process::id(),
        line!()
    ));
    std::fs::write(&path, b"abcdefghijklmnopqrstuvwxyz").unwrap();
    let spans = [(20, 3), (0, 3), (10, 2), (6, 0)];
    let actual = read_object_ranges(&path, &spans, 8).unwrap();
    assert_eq!(
        actual,
        vec![b"uvw".to_vec(), b"abc".to_vec(), b"kl".to_vec(), Vec::new()]
    );
    let overlapping = read_object_ranges(&path, &[(4, 4), (6, 4)], 8).unwrap();
    assert_eq!(overlapping, vec![b"efgh".to_vec(), b"ghij".to_vec()]);
    std::fs::remove_file(path).unwrap();
}
