use super::*;

#[test]
fn handles_all_signed_and_unsigned_boundaries_and_nulls() {
    for (kind, values, expected) in [
        ("Int8", "-128\n-1\n0\n127\n", 4),
        ("UInt8", "0\n1\n255\n", 3),
        ("Nullable(Int16)", "-32768\n-1\n32767\n\\N\n", 3),
        ("Nullable(UInt16)", "0\n65535\n\\N\n", 2),
    ] {
        let wire = format!("value\n{values}").into_bytes();
        assert_eq!(
            merge_distinct_bitmap16(&[(10, wire.clone()), (10, wire)], "value", kind).unwrap(),
            format!("count(DISTINCT value)\n{expected}\n").into_bytes(),
            "{kind}",
        );
    }
}

#[test]
fn covers_all_sixteen_bit_values_without_linear_key_storage() {
    let mut body = b"value\n".to_vec();
    for value in 0..=u16::MAX {
        body.extend_from_slice(format!("{value}\n").as_bytes());
    }
    assert_eq!(
        merge_distinct_bitmap16(&[(70_000, body.clone()), (70_000, body)], "value", "UInt16",)
            .unwrap(),
        b"count(DISTINCT value)\n65536\n"
    );
}

#[test]
fn rejects_invalid_widths_malformed_values_and_capped_peers() {
    for (kind, invalid) in [
        ("UInt8", "256"),
        ("Int8", "-129"),
        ("UInt16", "65536"),
        ("Int16", "32768"),
        ("Int16", "not-an-integer"),
    ] {
        let wire = format!("id\n{invalid}\n").into_bytes();
        assert_eq!(
            merge_distinct_bitmap16(&[(10, wire)], "id", kind),
            Err(MergeError::InvalidDistinctCount),
        );
    }
    assert_eq!(
        merge_distinct_bitmap16(&[(1, b"id\n12\n".to_vec())], "id", "UInt8"),
        Err(MergeError::ShardRowLimit),
    );
    assert_eq!(
        merge_distinct_bitmap16(&[(10, b"id\n\\N\n".to_vec())], "id", "UInt16").unwrap(),
        b"count(DISTINCT id)\n0\n"
    );
    assert_eq!(
        merge_distinct_bitmap16(&[(10, b"id\n12\n".to_vec())], "id", "UInt32"),
        Err(MergeError::InvalidDistinctCount),
    );
}
