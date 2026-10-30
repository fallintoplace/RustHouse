use super::*;

#[test]
fn merges_large_high_fanout_prefixes_without_row_materialization() {
    let shards = (0..24).map(|shard| {
        let mut output = b"id\n".to_vec();
        for row in 0..2026 {
            output.extend_from_slice(format!("{}\n", row * 24 + shard).as_bytes());
        }
        (4096, output)
    }).collect::<Vec<_>>();
    let actual = merge_sorted_integer_page(&shards, "id", "UInt64", false, 1999, 27, 27).unwrap();
    let mut expected = b"id\n".to_vec();
    for value in 1999..2026 {
        expected.extend_from_slice(format!("{value}\n").as_bytes());
    }
    assert_eq!(actual, expected);
}

#[test]
fn validates_all_shards_before_returning_even_when_page_is_empty() {
    let bad = [(5, b"id\n1\n".to_vec()), (5, b"id\n9\n3\n".to_vec())];
    assert_eq!(
        merge_sorted_integer_page(&bad, "id", "UInt64", false, 0, 0, 100),
        Err(MergeError::InvalidSortedScan)
    );
}
