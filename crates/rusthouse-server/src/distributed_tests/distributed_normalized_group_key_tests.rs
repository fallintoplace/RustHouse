use super::*;

#[test]
fn equivalent_tsv_escapes_merge_as_one_count_group() {
    let shards = [
        (10, b"key\tcount\nfirst\\tsecond\t2\n".to_vec()),
        (10, b"key\tcount\nfirst\\x09second\t3\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_counts(&shards, 10).unwrap(),
        b"key\tcount\nfirst\\tsecond\t5\n",
    );
}

#[test]
fn null_group_keys_do_not_collide_with_literal_backslash_n() {
    let shards = [
        (10, b"key\tcount\n\\N\t1\n".to_vec()),
        (10, b"key\tcount\n\\\\N\t2\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_counts(&shards, 10).unwrap(),
        b"key\tcount\n\\N\t1\n\\\\N\t2\n",
    );
}

#[test]
fn equivalent_tsv_escapes_merge_grouped_sums() {
    let shards = [
        (10, b"key\tsum(amount)\nfirst\\tsecond\t2\n".to_vec()),
        (10, b"key\tsum(amount)\nfirst\\x09second\t3\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_sums(&shards, 10, "Int64", "key", "amount").unwrap(),
        b"key\tsum(amount)\nfirst\\tsecond\t5\n",
    );
}
