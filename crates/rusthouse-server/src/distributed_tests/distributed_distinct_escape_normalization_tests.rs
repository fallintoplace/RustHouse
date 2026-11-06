use super::*;

#[test]
fn counts_equivalent_tsv_escapes_only_once_across_shards() {
    let shards = [
        (10, b"word\nfirst\\tsecond\n".to_vec()),
        (10, b"word\nfirst\\x09second\n".to_vec()),
        (10, b"word\n\\N\n".to_vec()),
    ];
    assert_eq!(
        merge_distinct_counts(&shards, "word").unwrap(),
        b"count(DISTINCT word)\n1\n",
    );
}

#[test]
fn refuses_malformed_tsv_escapes_in_distinct_values() {
    assert_eq!(
        merge_distinct_counts(&[(10, b"word\ninvalid\\q\n".to_vec())], "word"),
        Err(MergeError::InvalidDistinctCount),
    );
}
