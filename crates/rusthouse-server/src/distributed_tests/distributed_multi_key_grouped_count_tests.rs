use super::*;

#[test]
fn merges_composite_keys_across_disjoint_shards() {
    let sql = "SELECT region, category, count(*) FROM events GROUP BY region, category";
    let parsed = parse_select(sql).unwrap();
    assert!(is_safe_grouped_multi_count(&parsed));
    let Projection::GroupedMultiCount { columns } = parsed.projection else {
        panic!("expected grouped multi-count")
    };
    let shards = [
        (
            10,
            b"region\tcategory\tcount\nwest\ta\t2\neast\tb\t1\n".to_vec(),
        ),
        (
            10,
            b"region\tcategory\tcount\nwest\ta\t3\nwest\tb\t4\n".to_vec(),
        ),
    ];
    assert_eq!(
        merge_grouped_multi_counts(&shards, &columns, 10).unwrap(),
        b"region\tcategory\tcount\nwest\ta\t5\neast\tb\t1\nwest\tb\t4\n"
    );
}

#[test]
fn distinguishes_null_from_literal_backslash_n_and_decodes_escapes() {
    let keys = vec!["a".to_owned(), "b".to_owned()];
    let shards = [
        (
            10,
            b"a\tb\tcount\n\\N\tx\t2\n\\\\N\tx\t1\na\t\\x41\t3\n".to_vec(),
        ),
        (10, b"a\tb\tcount\n\\N\tx\t1\na\tA\t4\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_multi_counts(&shards, &keys, 10).unwrap(),
        b"a\tb\tcount\n\\N\tx\t3\n\\\\N\tx\t1\na\t\\x41\t7\n"
    );
}

#[test]
fn refuses_wrong_tuple_width_capped_and_overflowing_shard_results() {
    let keys = vec!["region".to_owned(), "category".to_owned()];
    for body in [
        b"region\tcategory\tcount\nwest\t2\n".as_slice(),
        b"region\tcategory\tcount\nwest\ta\t2\textra\n",
        b"region\tcategory\tcount\nwest\ta\t-1\n",
        b"region\tcategory\tcount\nwest\tbad\\q\t2\n",
    ] {
        assert_eq!(
            merge_grouped_multi_counts(&[(10, body.to_vec())], &keys, 10),
            Err(MergeError::InvalidGroupedCount)
        );
    }
    assert_eq!(
        merge_grouped_multi_counts(
            &[(1, b"region\tcategory\tcount\nwest\ta\t1\n".to_vec())],
            &keys,
            10
        ),
        Err(MergeError::ShardRowLimit)
    );
    let overflow = [
        (
            10,
            b"region\tcategory\tcount\nwest\ta\t18446744073709551615\n".to_vec(),
        ),
        (10, b"region\tcategory\tcount\nwest\ta\t1\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_multi_counts(&overflow, &keys, 10),
        Err(MergeError::CountOverflow)
    );
}
