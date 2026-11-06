use super::*;

#[test]
fn rejects_corrupt_wire_escapes_in_all_rows_including_skipped_pages() {
    let invalid = vec![(16, b"name\nfirst\nsecond\\q\n".to_vec())];
    assert_eq!(merge_tsv(&invalid, 16), Err(MergeError::InvalidResponse));
    assert_eq!(
        merge_distinct_tsv(&invalid, 16, 0, None),
        Err(MergeError::InvalidResponse)
    );
    assert_eq!(
        merge_tsv_page(&invalid, 16, 0, Some(1)),
        Err(MergeError::InvalidResponse),
    );
    assert_eq!(
        merge_tsv_page(&invalid, 16, 2, Some(0)),
        Err(MergeError::InvalidResponse),
    );
}

#[test]
fn null_marker_is_not_a_malformed_escape() {
    let shard = vec![(16, b"id\tvalue\n1\t\\N\n2\t\\\\N\n".to_vec())];
    assert_eq!(merge_tsv(&shard, 16).unwrap(), shard[0].1);
    assert_eq!(
        merge_tsv_page(&shard, 16, 1, Some(1)).unwrap(),
        b"id\tvalue\n2\t\\\\N\n",
    );
}

#[test]
fn accepts_valid_escaped_tab_newline_and_hex_sequences() {
    let body = vec![(16, b"name\tvalue\na\\tb\tx\\x41\na\\nb\t\\\\\n".to_vec())];
    assert_eq!(merge_tsv(&body, 16).unwrap(), body[0].1);
    assert_eq!(
        merge_tsv_page(&body, 16, 1, Some(1)).unwrap(),
        b"name\tvalue\na\\nb\t\\\\\n"
    );
}
