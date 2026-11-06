use super::*;

#[test]
fn supports_exact_integer_sum_squares_without_global_clauses() {
    for sql in [
        "SELECT sumSquares(amount) FROM events",
        "SELECT sumSquares(amount) FROM events WHERE id > 3",
    ] {
        assert!(is_safe_sum_squares(&parse_select(sql).unwrap()));
    }
    for sql in [
        "SELECT sumSquares(amount) FROM events LIMIT 1",
        "SELECT sumSquares(amount) FROM numbers(10)",
        "SELECT sumSquares(amount) FROM events OFFSET 1",
    ] {
        assert!(parse_select(sql).map_or(true, |query| !is_safe_sum_squares(&query)));
    }
}

#[test]
fn combines_shard_partial_squares_without_rounding_or_null_identity() {
    let values = [
        (10, b"sumSquares(amount)\n9\n".to_vec()),
        (10, b"sumSquares(amount)\nNULL\n".to_vec()),
        (10, b"sumSquares(amount)\n16\n".to_vec()),
    ];
    assert_eq!(
        merge_integer_sums(&values, "Nullable(Int64)").unwrap(),
        b"sumSquares(amount)\n25\n"
    );
    assert_eq!(
        merge_integer_sums(&values[1..2], "Int64").unwrap(),
        b"sumSquares(amount)\nNULL\n"
    );
}
