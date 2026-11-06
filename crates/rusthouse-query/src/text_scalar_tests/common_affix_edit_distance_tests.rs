use super::*;

#[test]
fn long_common_prefix_and_suffix_do_not_exhaust_the_work_budget() {
    let prefix = "a".repeat(65_536);
    let suffix = "z".repeat(65_536);
    let left = format!("{prefix}X{suffix}");
    let right = format!("{prefix}Y{suffix}");
    assert_eq!(
        bounded_edit_distance(left.as_bytes(), right.as_bytes()).unwrap(),
        1
    );
    assert_eq!(
        bounded_edit_distance(right.as_bytes(), left.as_bytes()).unwrap(),
        1
    );

    let prefix = "🚀".repeat(8192);
    let suffix = "é".repeat(8192);
    let left = format!("{prefix}猫{suffix}");
    let right = format!("{prefix}犬{suffix}");
    assert_eq!(bounded_utf8_edit_distance(&left, &right).unwrap(), 1);
}

#[test]
fn common_edge_trimming_preserves_insertions_and_deletions() {
    for (left, right, distance) in [
        ("abcdef", "abcXdef", 1),
        ("abcXdef", "abcdef", 1),
        ("kitten", "sitting", 3),
        ("", "three", 5),
        ("foobarbaz", "foobaz", 3),
        ("aaaa", "aaa", 1),
    ] {
        assert_eq!(
            bounded_edit_distance(left.as_bytes(), right.as_bytes()).unwrap(),
            distance
        );
        assert_eq!(bounded_utf8_edit_distance(left, right).unwrap(), distance);
    }
}

#[test]
fn completely_different_large_strings_keep_the_work_limit() {
    let left = "x".repeat(2048);
    let right = "y".repeat(2048);
    assert!(bounded_edit_distance(left.as_bytes(), right.as_bytes()).is_err());
    assert!(bounded_utf8_edit_distance(&left, &right).is_err());
}
