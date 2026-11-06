use super::*;

#[test]
fn rewrites_multi_grouped_distinct_without_changing_where_predicate() {
    let sql = "SELECT region, category, count(DISTINCT label) FROM events WHERE id > 3 GROUP BY region, category";
    let parsed = parse_select(sql).unwrap();
    assert!(is_safe_multi_grouped_distinct(&parsed));
    assert_eq!(
        multi_grouped_distinct_shard_sql(sql, &parsed).as_deref(),
        Some("SELECT DISTINCT region, category, label FROM events WHERE id > 3"),
    );
}

#[test]
fn merges_duplicate_pairs_but_retains_null_only_groups() {
    let columns = vec!["region".into(), "category".into()];
    let shards = [
        (
            10,
            b"region\tcategory\tlabel\nwest\ta\tx\nwest\ta\ty\neast\tb\t\\N\n".to_vec(),
        ),
        (
            10,
            b"region\tcategory\tlabel\nwest\ta\ty\nwest\ta\tz\neast\tb\t\\N\n".to_vec(),
        ),
    ];
    assert_eq!(
        merge_multi_grouped_distinct_pairs(&shards, &columns, "label", 10).unwrap(),
        b"region\tcategory\tcountDistinct(label)\nwest\ta\t3\neast\tb\t0\n",
    );
}

#[test]
fn encoded_string_keys_and_nulls_do_not_alias() {
    let columns = vec!["a".into(), "b".into()];
    let shards = [
        (
            10,
            b"a\tb\tv\n\\N\tx\t1\n\\\\N\tx\t1\na\t\\x41\t2\n".to_vec(),
        ),
        (10, b"a\tb\tv\n\\N\tx\t1\na\tA\t2\n".to_vec()),
    ];
    assert_eq!(
        merge_multi_grouped_distinct_pairs(&shards, &columns, "v", 10).unwrap(),
        b"a\tb\tcountDistinct(v)\n\\N\tx\t1\n\\\\N\tx\t1\na\t\\x41\t1\n",
    );
}

#[test]
fn corrupt_width_and_invalid_escape_fail_the_entire_query() {
    let columns = vec!["a".into(), "b".into()];
    for body in [
        b"a\tb\tv\nA\t1\n".as_slice(),
        b"a\tb\tv\nA\tB\tx\ty\n",
        b"a\tb\tv\nA\tB\tbad\\q\n",
    ] {
        assert_eq!(
            merge_multi_grouped_distinct_pairs(&[(10, body.to_vec())], &columns, "v", 10),
            Err(MergeError::InvalidGroupedCount)
        );
    }
    assert_eq!(
        merge_multi_grouped_distinct_pairs(
            &[(1, b"a\tb\tv\nA\tB\t1\n".to_vec())],
            &columns,
            "v",
            10,
        ),
        Err(MergeError::ShardRowLimit),
    );
}
