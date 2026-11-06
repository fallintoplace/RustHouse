use super::*;

#[test]
fn merges_nullable_string_counts_for_shared_keys() {
    let sql = "SELECT category, count(label) FROM events GROUP BY category";
    let parsed = parse_select(sql).unwrap();
    assert!(is_safe_grouped_sum(&parsed));
    let shards = [
        (10, b"category\tcount(label)\na\t2\nb\t0\n".to_vec()),
        (10, b"category\tcount(label)\na\t3\nc\t1\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_sums_query(
            &shards,
            10,
            "Nullable(String)",
            "category",
            "label",
            Some(&parsed)
        )
        .unwrap(),
        b"category\tcount(label)\na\t5\nb\t0\nc\t1\n"
    );
}

#[test]
fn global_having_applies_after_combining_nullable_column_counts() {
    let sql =
        "SELECT category, count(label) FROM events GROUP BY category HAVING count(label) > 3";
    let parsed = parse_select(sql).unwrap();
    assert!(is_safe_grouped_sum(&parsed));
    assert_eq!(
        grouped_count_shard_sql(sql, &parsed).as_deref(),
        Some("SELECT category, count(label) FROM events GROUP BY category"),
    );
    let shards = [
        (10, b"category\tcount(label)\na\t2\nb\t1\n".to_vec()),
        (10, b"category\tcount(label)\na\t3\nb\t1\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_sums_query(
            &shards,
            10,
            "String",
            "category",
            "label",
            Some(&parsed)
        )
        .unwrap(),
        b"category\tcount(label)\na\t5\n"
    );
}

#[test]
fn rejects_negative_counts_and_capped_shard_responses() {
    let parsed =
        parse_select("SELECT category, count(label) FROM events GROUP BY category").unwrap();
    assert_eq!(
        merge_grouped_integer_sums_query(
            &[(10, b"category\tcount(label)\na\t-1\n".to_vec())],
            10,
            "String",
            "category",
            "label",
            Some(&parsed)
        ),
        Err(MergeError::InvalidGroupedSum),
    );
    assert_eq!(
        merge_grouped_integer_sums_query(
            &[(1, b"category\tcount(label)\na\t2\n".to_vec())],
            10,
            "String",
            "category",
            "label",
            Some(&parsed)
        ),
        Err(MergeError::ShardRowLimit),
    );
}
