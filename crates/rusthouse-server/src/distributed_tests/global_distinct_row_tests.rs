use super::*;

fn shard(cap: usize, body: &str) -> (usize, Vec<u8>) {
    (cap, body.as_bytes().to_vec())
}

#[test]
fn exact_distinct_rows_preserve_multi_column_and_null_semantics() {
    let shards = [
        shard(8, "id\tregion\n1\teu\n2\t\\N\n1\teu\n"),
        shard(8, "id\tregion\n2\t\\N\n3\tus\n"),
    ];
    assert_eq!(
        merge_distinct_tsv(&shards, 10, 0, None).unwrap(),
        b"id\tregion\n1\teu\n2\t\\N\n3\tus\n"
    );
    assert_eq!(
        merge_distinct_tsv(&shards, 1, 1, Some(1)).unwrap(),
        b"id\tregion\n2\t\\N\n"
    );
    assert!(merge_distinct_tsv(&shards, 2, 0, None).is_err());
    assert!(merge_distinct_tsv(&[shard(2, "id\n1\n2\n")], 2, 0, None).is_err());
    assert!(merge_distinct_tsv(&[shard(10, "id\tname\n1\n")], 10, 0, None).is_err());
}

#[test]
fn distributed_distinct_policy_accepts_only_unordered_rows() {
    assert!(is_safe_distinct_scan(
        &parse_select("SELECT DISTINCT id, region FROM events LIMIT 10 OFFSET 2").unwrap()
    ));
    assert!(!is_safe_distinct_scan(
        &parse_select("SELECT DISTINCT id FROM events ORDER BY id LIMIT 10").unwrap()
    ));
    assert!(!is_safe_distinct_scan(
        &parse_select("SELECT id FROM events").unwrap()
    ));
}
