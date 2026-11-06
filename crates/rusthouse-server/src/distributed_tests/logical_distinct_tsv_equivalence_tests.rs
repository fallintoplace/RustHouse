use super::*;

#[test]
fn distinct_and_pagination_use_decoded_field_values() {
    let shards = [
        (16, b"id\tvalue\n1\tA\n2\t\\x41\n3\t\\t\n".to_vec()),
        (16, b"id\tvalue\n1\t\\x41\n3\t\\x09\n2\tA\n".to_vec()),
    ];
    assert_eq!(
        merge_distinct_tsv(&shards, 16, 0, None).unwrap(),
        b"id\tvalue\n1\tA\n2\t\\x41\n3\t\\t\n",
    );
    assert_eq!(
        merge_distinct_tsv(&shards, 2, 1, Some(2)).unwrap(),
        b"id\tvalue\n2\t\\x41\n3\t\\t\n",
    );
}

#[test]
fn null_marker_is_not_the_same_value_as_a_literal_backslash_n() {
    let shards = [(16, b"v\n\\N\n\\\\N\n\\x5cN\n".to_vec())];
    assert_eq!(
        merge_distinct_tsv(&shards, 16, 0, None).unwrap(),
        b"v\n\\N\n\\\\N\n",
    );
    assert_eq!(
        merge_distinct_tsv(&[(16, b"v\nvalid\n\\q\n".to_vec())], 1, 0, Some(1)),
        Err(MergeError::InvalidResponse),
    );
}
