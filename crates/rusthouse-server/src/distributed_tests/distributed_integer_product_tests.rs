use super::*;

fn shard(body: &str) -> (usize, Vec<u8>) {
    (10, body.as_bytes().to_vec())
}

#[test]
fn multiplies_signed_and_unsigned_partial_products() {
    let values = [
        shard("product(amount)\n-3\n"),
        shard("product(amount)\n4\n"),
        shard("product(amount)\nNULL\n"),
    ];
    assert_eq!(
        merge_integer_products(&values, "Nullable(Int64)").unwrap(),
        b"product(amount)\n-12\n"
    );
    let values = [
        shard("product(amount)\n0\n"),
        shard("product(amount)\n18446744073709551615\n"),
    ];
    assert_eq!(
        merge_integer_products(&values, "UInt64").unwrap(),
        b"product(amount)\n0\n"
    );
    assert_eq!(
        merge_integer_products(&[shard("product(amount)\nNULL\n")], "Int64").unwrap(),
        b"product(amount)\nNULL\n"
    );
}

#[test]
fn rejects_overflow_malformed_values_and_mismatched_headers() {
    let values = [
        shard("product(amount)\n340282366920938463463374607431768211455\n"),
        shard("product(amount)\n2\n"),
    ];
    assert_eq!(
        merge_integer_products(&values, "UInt128"),
        Err(MergeError::ProductOverflow),
    );
    for invalid in [
        "product(amount)\n-1\n",
        "product(amount)\n1\n2\n",
        "product(amount)\n",
    ] {
        assert_eq!(
            merge_integer_products(&[shard(invalid)], "UInt64"),
            Err(MergeError::InvalidProduct),
        );
    }
    assert_eq!(
        merge_integer_products(
            &[shard("product(amount)\n2\n"), shard("product(other)\n3\n")],
            "UInt64",
        ),
        Err(MergeError::SchemaMismatch),
    );
}

#[test]
fn product_requires_unpaged_named_table() {
    assert!(is_safe_product(
        &parse_select("SELECT product(amount) FROM events").unwrap()
    ));
    assert!(!is_safe_product(
        &parse_select("SELECT product(amount) FROM events LIMIT 1").unwrap()
    ));
}
