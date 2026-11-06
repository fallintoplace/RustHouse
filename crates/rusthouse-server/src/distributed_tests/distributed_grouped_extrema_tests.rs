use super::*;

#[test]
fn merges_nullable_groups_exactly_before_global_having() {
    let sql = "SELECT label, min(amount) FROM events GROUP BY label HAVING min(amount) < 10";
    let parsed = parse_select(sql).unwrap();
    assert!(is_safe_grouped_extremum(&parsed));
    let shards = [
        (10, b"label\tmin(amount)\na\t12\nb\tNULL\n".to_vec()),
        (10, b"label\tmin(amount)\na\t4\nb\t8\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_aggregate_query(
            &shards,
            10,
            "Nullable(Int64)",
            "label",
            "amount",
            AggregateFunction::Min,
            Some(&parsed),
        )
        .unwrap(),
        b"label\tmin(amount)\na\t4\nb\t8\n"
    );
}

#[test]
fn computes_signed_max_and_rejects_corrupt_peer_headers() {
    let sql = "SELECT label, max(amount) FROM events GROUP BY label";
    let parsed = parse_select(sql).unwrap();
    let shards = [
        (10, b"label\tmax(amount)\na\t-20\n".to_vec()),
        (10, b"label\tmax(amount)\na\t-3\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_aggregate_query(
            &shards,
            10,
            "Int64",
            "label",
            "amount",
            AggregateFunction::Max,
            Some(&parsed),
        )
        .unwrap(),
        b"label\tmax(amount)\na\t-3\n"
    );
    assert_eq!(
        merge_grouped_integer_aggregate_query(
            &[(10, b"label\tsum(amount)\na\t4\n".to_vec())],
            10,
            "Int64",
            "label",
            "amount",
            AggregateFunction::Max,
            Some(&parsed),
        ),
        Err(MergeError::SchemaMismatch),
    );
}
