use super::*;

#[test]
fn rewrites_grouped_distinct_without_dropping_where_conditions() {
    let sql =
        "SELECT category, count(DISTINCT label) FROM events WHERE id >= 10 GROUP BY category";
    let parsed = parse_select(sql).unwrap();
    assert!(is_safe_grouped_distinct(&parsed));
    assert_eq!(
        grouped_distinct_shard_sql(sql, &parsed).as_deref(),
        Some("SELECT DISTINCT category, label FROM events WHERE id >= 10"),
    );
    let unsafe_sql =
        "SELECT category, count(DISTINCT label) FROM events GROUP BY category LIMIT 2";
    assert!(!is_safe_grouped_distinct(
        &parse_select(unsafe_sql).unwrap()
    ));
}

#[test]
fn counts_shared_pairs_once_but_retains_null_only_groups() {
    let shards = [
        (10, b"category\tlabel\na\tx\na\ty\nb\t\\N\n".to_vec()),
        (10, b"category\tlabel\na\ty\na\tz\nc\tvalue\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_distinct_pairs(&shards, "category", "label", 10).unwrap(),
        b"category\tcountDistinct(label)\na\t3\nb\t0\nc\t1\n",
    );
}

#[test]
fn escaped_values_use_logical_equality_and_null_groups_are_distinct() {
    let shards = [
        (
            10,
            b"category\tlabel\n\\N\tx\n\\\\N\tx\na\t\\x41\n".to_vec(),
        ),
        (10, b"category\tlabel\n\\N\tx\na\tA\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_distinct_pairs(&shards, "category", "label", 10).unwrap(),
        b"category\tcountDistinct(label)\n\\N\t1\n\\\\N\t1\na\t1\n",
    );
}

#[test]
fn invalid_row_width_or_escapes_do_not_produce_partial_counts() {
    for body in [
        b"category\tlabel\na\n".as_slice(),
        b"category\tlabel\na\tx\textra\n",
        b"category\tlabel\na\tbad\\q\n",
    ] {
        assert_eq!(
            merge_grouped_distinct_pairs(&[(10, body.to_vec())], "category", "label", 10),
            Err(MergeError::InvalidGroupedCount),
        );
    }
    assert_eq!(
        merge_grouped_distinct_pairs(
            &[(1, b"category\tlabel\na\tx\n".to_vec())],
            "category",
            "label",
            10
        ),
        Err(MergeError::ShardRowLimit)
    );
}
