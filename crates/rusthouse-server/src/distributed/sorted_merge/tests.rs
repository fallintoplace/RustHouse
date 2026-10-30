use super::*;
fn shard(cap: usize, data: &str) -> (usize, Vec<u8>) {
    (cap, data.as_bytes().to_vec())
}

#[test]
fn global_integer_topk_merges_peer_prefixes_and_paginates() {
    let shards = [
        shard(10, "id\n1\n4\n7\n"),
        shard(10, "id\n2\n3\n8\n"),
        shard(10, "id\n0\n5\n9\n"),
    ];
    assert_eq!(
        merge_sorted_integer_page(&shards, "id", "UInt64", false, 2, 3, 10).unwrap(),
        b"id\n2\n3\n4\n"
    );
    let desc = [shard(10, "id\n9\n5\n1\n"), shard(10, "id\n8\n6\n2\n")];
    assert_eq!(
        merge_sorted_integer_page(&desc, "id", "Int64", true, 1, 3, 10).unwrap(),
        b"id\n8\n6\n5\n"
    );
}

#[test]
fn rejects_unsorted_shard_rows_and_insufficient_caps() {
    assert_eq!(
        merge_sorted_integer_page(&[shard(10, "id\n3\n2\n")], "id", "UInt64", false, 0, 2, 10),
        Err(MergeError::InvalidSortedScan)
    );
    assert_eq!(
        merge_sorted_integer_page(&[shard(2, "id\n1\n2\n")], "id", "UInt64", false, 0, 2, 10),
        Err(MergeError::InvalidSortedScan)
    );
}

#[test]
fn rejects_noncanonical_peer_integer_keys() {
    for invalid in ["id\n+1\n", "id\n01\n", "id\n-0\n", "id\n-01\n"] {
        assert_eq!(
            merge_sorted_integer_page(&[shard(5, invalid)], "id", "Int64", false, 0, 1, 1),
            Err(MergeError::InvalidSortedScan),
            "{invalid:?}"
        );
    }
    assert_eq!(
        merge_sorted_integer_page(&[shard(5, "id\n01\n")], "id", "UInt64", false, 0, 1, 1),
        Err(MergeError::InvalidSortedScan)
    );
}

#[test]
fn unsigned_merge_preserves_uint128_above_signed_range() {
    let shards = [
        shard(5, "id\n340282366920938463463374607431768211454\n"),
        shard(5, "id\n340282366920938463463374607431768211455\n"),
    ];
    assert_eq!(
        merge_sorted_integer_page(&shards, "id", "UInt128", true, 0, 2, 10).unwrap(),
        b"id\n340282366920938463463374607431768211455\n340282366920938463463374607431768211454\n"
    );
}
