use super::*;

#[test]
fn combines_wide_integer_duplicates_and_nullable_rows() {
    for (kind, rows, expected) in [
        ("Int64", "-9223372036854775808\n0\n9223372036854775807\n", 3),
        ("UInt64", "0\n18446744073709551615\n", 2),
        (
            "Nullable(Int128)",
            "-170141183460469231731687303715884105728\n0\n170141183460469231731687303715884105727\n\\N\n",
            3,
        ),
        (
            "Nullable(UInt128)",
            "0\n340282366920938463463374607431768211455\n\\N\n",
            2,
        ),
    ] {
        let wire = format!("id\n{rows}").into_bytes();
        assert_eq!(
            merge_distinct_wide_integers(&[(20, wire.clone()), (20, wire)], "id", kind)
                .unwrap(),
            format!("count(DISTINCT id)\n{expected}\n").into_bytes(),
            "{kind}"
        );
    }
}

#[test]
fn refuses_bad_wide_integer_encodings_and_incorrect_widths() {
    for (kind, value) in [
        ("Int64", "9223372036854775808"),
        ("UInt64", "-1"),
        ("UInt64", "18446744073709551616"),
        ("Int128", "170141183460469231731687303715884105728"),
        ("UInt128", "340282366920938463463374607431768211456"),
        ("Int64", "+1"),
        ("Int64", "01"),
        ("Int64", "-0"),
        ("Int64", " 1"),
    ] {
        assert_eq!(
            merge_distinct_wide_integers(
                &[(20, format!("id\n{value}\n").into_bytes())],
                "id",
                kind,
            ),
            Err(MergeError::InvalidDistinctCount),
            "{kind} {value}"
        );
    }
}

#[test]
fn honors_peer_row_caps_and_exact_one_hundred_thousand_key_budget() {
    let values = (0..100_000u64)
        .map(|value| format!("{value}\n"))
        .collect::<String>();
    let wire = format!("id\n{values}").into_bytes();
    assert_eq!(
        merge_distinct_wide_integers(
            &[(100_002, wire.clone()), (100_002, wire.clone())],
            "id",
            "UInt64",
        )
        .unwrap(),
        b"count(DISTINCT id)\n100000\n"
    );
    assert_eq!(
        merge_distinct_wide_integers(&[(100_000, wire)], "id", "UInt64"),
        Err(MergeError::ShardRowLimit)
    );
    let additional = vec![
        (100_002, format!("id\n{values}").into_bytes()),
        (3, b"id\n100000\n".to_vec()),
    ];
    assert_eq!(
        merge_distinct_wide_integers(&additional, "id", "UInt64"),
        Err(MergeError::CoordinatorLimit)
    );
}
