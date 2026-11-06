use super::bounded_utf8_edit_distance;

#[test]
fn common_multibyte_edges_are_trimmed_without_changing_distance() {
    for (left, right, expected) in [
        ("🎉éabc界", "🎉éaxc界", 1),
        ("🎉éabc界", "🎉éab界", 1),
        ("🎉éabc界", "🎉éabcd界", 1),
        ("🙂🙂", "🙂", 1),
        ("é", "e", 1),
        ("e\u{301}", "é", 2),
        ("hello", "world", 4),
    ] {
        assert_eq!(bounded_utf8_edit_distance(left, right).unwrap(), expected);
        assert_eq!(bounded_utf8_edit_distance(right, left).unwrap(), expected);
    }
}

#[test]
fn long_identical_edges_leave_only_small_edits_to_allocate() {
    let prefix = "🎉".repeat(100_000);
    let suffix = "界".repeat(100_000);
    let left = format!("{prefix}a{suffix}");
    let right = format!("{prefix}b{suffix}");
    assert_eq!(bounded_utf8_edit_distance(&left, &right).unwrap(), 1);
}
