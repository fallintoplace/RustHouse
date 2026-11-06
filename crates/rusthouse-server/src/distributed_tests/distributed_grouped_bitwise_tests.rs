use super::*;

#[test]
fn combines_signed_and_nullable_bitwise_groups_across_shards() {
    let select =
        parse_select("SELECT region, groupBitXor(bits) FROM events GROUP BY region").unwrap();
    assert!(is_safe_grouped_bitwise(&select));
    let shards = [
        (10, b"region\tgroupBitXor(bits)\na\t3\nb\tNULL\n".to_vec()),
        (10, b"region\tgroupBitXor(bits)\na\t5\nb\t7\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_bitwise_query(
            &shards,
            10,
            "Nullable(UInt64)",
            "region",
            "bits",
            AggregateFunction::GroupBitXor,
            Some(&select),
        )
        .unwrap(),
        b"region\tgroupBitXor(bits)\na\t6\nb\t7\n"
    );
    let and = [
        (10, b"region\tgroupBitAnd(bits)\na\t-1\n".to_vec()),
        (10, b"region\tgroupBitAnd(bits)\na\t7\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_bitwise_query(
            &and,
            10,
            "Int64",
            "region",
            "bits",
            AggregateFunction::GroupBitAnd,
            None,
        )
        .unwrap(),
        b"region\tgroupBitAnd(bits)\na\t7\n"
    );
}

#[test]
fn rejects_bad_grouped_bitwise_headers() {
    assert_eq!(
        merge_grouped_integer_bitwise_query(
            &[(10, b"region\tsum(bits)\na\t3\n".to_vec())],
            10,
            "UInt64",
            "region",
            "bits",
            AggregateFunction::GroupBitOr,
            None,
        ),
        Err(MergeError::SchemaMismatch),
    );
}
