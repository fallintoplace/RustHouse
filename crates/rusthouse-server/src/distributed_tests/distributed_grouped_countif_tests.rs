use super::*;

#[test]
fn reduces_signed_input_countif_as_unsigned_counts() {
    let sql = "SELECT category, countIf(amount) FROM events GROUP BY category";
    let parsed = parse_select(sql).unwrap();
    assert!(is_safe_grouped_sum(&parsed));
    let shards = [
        (10, b"category\tcountIf(amount)\na\t2\nb\t0\n".to_vec()),
        (10, b"category\tcountIf(amount)\na\t3\nc\t1\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_sums_query(
            &shards,
            10,
            "Int64",
            "category",
            "amount",
            Some(&parsed)
        )
        .unwrap(),
        b"category\tcountIf(amount)\na\t5\nb\t0\nc\t1\n"
    );
}

#[test]
fn applies_having_after_global_reduction() {
    let sql = "SELECT category, countIf(amount) FROM events GROUP BY category HAVING countIf(amount) > 3";
    let parsed = parse_select(sql).unwrap();
    assert!(is_safe_grouped_sum(&parsed));
    assert_eq!(
        grouped_count_shard_sql(sql, &parsed).as_deref(),
        Some("SELECT category, countIf(amount) FROM events GROUP BY category"),
    );
    let shards = [
        (10, b"category\tcountIf(amount)\na\t2\nb\t1\n".to_vec()),
        (10, b"category\tcountIf(amount)\na\t3\nb\t1\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_sums_query(
            &shards,
            10,
            "UInt8",
            "category",
            "amount",
            Some(&parsed)
        )
        .unwrap(),
        b"category\tcountIf(amount)\na\t5\n"
    );
}

#[test]
fn rejects_malformed_partial_counts_and_total_overflow() {
    let parsed =
        parse_select("SELECT category, countIf(amount) FROM events GROUP BY category").unwrap();
    assert_eq!(
        merge_grouped_integer_sums_query(
            &[(10, b"category\tcountIf(amount)\na\t-1\n".to_vec())],
            10,
            "Int64",
            "category",
            "amount",
            Some(&parsed)
        ),
        Err(MergeError::InvalidGroupedSum),
    );
    let overflow = [
        (
            10,
            format!("category\tcountIf(amount)\na\t{}\n", u128::MAX).into_bytes(),
        ),
        (10, b"category\tcountIf(amount)\na\t1\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_sums_query(
            &overflow,
            10,
            "UInt64",
            "category",
            "amount",
            Some(&parsed)
        ),
        Err(MergeError::SumOverflow),
    );
}
