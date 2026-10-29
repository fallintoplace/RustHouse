use super::*;

#[test]
fn preserves_signed_duplicate_pages_across_shard_boundaries() {
    let shards = [
        (12, b"id\n-9\n-2\n-2\n7\n".to_vec()),
        (12, b"id\n-12\n-2\n0\n8\n".to_vec()),
        (12, b"id\n-10\n-2\n1\n99\n".to_vec()),
    ];
    assert_eq!(
        merge_sorted_integer_page(&shards, "id", "Int64", false, 2, 5, 5).unwrap(),
        b"id\n-9\n-2\n-2\n-2\n-2\n"
    );
    assert_eq!(
        merge_sorted_integer_page(&shards, "id", "Int64", false, 1, 0, 5).unwrap(),
        b"id\n"
    );
}
