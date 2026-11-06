use super::*;

#[test]
fn rejects_incomplete_and_excess_tsv_fields() {
    for row in ["1\n", "1\ttwo\tthree\n"] {
        let body = format!("id\tvalue\n{row}").into_bytes();
        assert_eq!(
            merge_tsv(&[(10, body)], 10),
            Err(MergeError::InvalidResponse),
        );
    }
    assert_eq!(
        merge_tsv(&[(10, b"id\tvalue\n1\t\n".to_vec())], 10).unwrap(),
        b"id\tvalue\n1\t\n",
    );
    // Even a malformed shard beyond the requested page must be rejected.
    let shards = [
        (10, b"id\tvalue\n1\tone\n".to_vec()),
        (10, b"id\tvalue\n2\n".to_vec()),
    ];
    assert_eq!(
        merge_tsv_page(&shards, 1, 0, Some(1)),
        Err(MergeError::InvalidResponse),
    );
}
