use super::*;

#[test]
fn rejects_out_of_range_peer_minmax_values() {
    for (kind, value) in [
        ("Int8", "128"),
        ("Int8", "-129"),
        ("UInt8", "256"),
        ("Nullable(UInt16)", "65536"),
        ("Int16", "32768"),
        ("UInt64", "18446744073709551616"),
        ("Int128", "170141183460469231731687303715884105728"),
    ] {
        let body = format!("min(amount)\n{value}\n");
        assert_eq!(
            merge_integer_extrema(&[(10, body.into_bytes())], kind, AggregateFunction::Min,),
            Err(MergeError::InvalidExtremum),
            "{kind}: {value}",
        );
    }
}

#[test]
fn accepts_signed_unsigned_and_nullable_schema_boundaries() {
    for (kind, value) in [
        ("Int8", "-128"),
        ("Int8", "127"),
        ("UInt8", "255"),
        ("Nullable(UInt16)", "65535"),
        ("Int16", "-32768"),
        ("Int128", "-170141183460469231731687303715884105728"),
        ("UInt128", "340282366920938463463374607431768211455"),
    ] {
        let body = format!("max(amount)\n{value}\n");
        assert!(
            merge_integer_extrema(&[(10, body.into_bytes())], kind, AggregateFunction::Max,)
                .is_ok(),
            "{kind}: {value}"
        );
    }
}
