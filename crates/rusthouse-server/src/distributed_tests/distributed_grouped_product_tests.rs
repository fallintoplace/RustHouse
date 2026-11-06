use super::*;

#[test]
fn reduces_integer_products_and_skips_empty_nullable_parts() {
    let select =
        parse_select("SELECT region, product(amount) FROM events GROUP BY region").unwrap();
    assert!(is_safe_grouped_product(&select));
    let shards = [
        (10, b"region\tproduct(amount)\na\t3\nb\tNULL\n".to_vec()),
        (10, b"region\tproduct(amount)\na\t4\nb\t2\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_products_query(
            &shards,
            10,
            "Nullable(Int64)",
            "region",
            "amount",
            Some(&select),
        )
        .unwrap(),
        b"region\tproduct(amount)\na\t12\nb\t2\n"
    );
}

#[test]
fn rejects_overflow_and_non_product_response_headers() {
    let select =
        parse_select("SELECT region, product(amount) FROM events GROUP BY region").unwrap();
    let shards = [
        (
            10,
            b"region\tproduct(amount)\na\t340282366920938463463374607431768211455\n".to_vec(),
        ),
        (10, b"region\tproduct(amount)\na\t2\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_products_query(
            &shards,
            10,
            "UInt128",
            "region",
            "amount",
            Some(&select),
        ),
        Err(MergeError::ProductOverflow),
    );
    assert_eq!(
        merge_grouped_integer_products_query(
            &[(10, b"region\tsum(amount)\na\t2\n".to_vec())],
            10,
            "UInt64",
            "region",
            "amount",
            Some(&select),
        ),
        Err(MergeError::SchemaMismatch),
    );
}
