use super::*;

#[test]
fn grouped_sum_squares_reduces_shared_keys_across_shards() {
    let sql = "SELECT category, sumSquares(amount) FROM events GROUP BY category";
    let select = parse_select(sql).unwrap();
    assert!(is_safe_grouped_sum(&select));
    let shards = [
        (
            10,
            b"category\tsumSquares(amount)\na\t9\nb\tNULL\n".to_vec(),
        ),
        (10, b"category\tsumSquares(amount)\na\t16\nb\t4\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_sums_query(
            &shards,
            10,
            "Int64",
            "category",
            "amount",
            Some(&select)
        )
        .unwrap(),
        b"category\tsumSquares(amount)\na\t25\nb\t4\n"
    );
}

#[test]
fn grouped_sum_squares_uses_global_having_after_reduction() {
    let sql = "SELECT category, sumSquares(amount) FROM events GROUP BY category HAVING sumSquares(amount) > 20";
    let select = parse_select(sql).unwrap();
    assert!(is_safe_grouped_sum(&select));
    let shards = [
        (10, b"category\tsumSquares(amount)\na\t9\nb\t16\n".to_vec()),
        (10, b"category\tsumSquares(amount)\na\t16\nb\t0\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_sums_query(
            &shards,
            10,
            "UInt64",
            "category",
            "amount",
            Some(&select)
        )
        .unwrap(),
        b"category\tsumSquares(amount)\na\t25\n"
    );
}
