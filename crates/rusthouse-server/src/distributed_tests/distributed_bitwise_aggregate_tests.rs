use super::*;

fn shard(value: &str) -> (usize, Vec<u8>) {
    (10, value.as_bytes().to_vec())
}

#[test]
fn merges_all_three_bitwise_functions() {
    let xor = [
        shard("groupBitXor(bits)\n3\n"),
        shard("groupBitXor(bits)\n5\n"),
    ];
    assert_eq!(
        merge_integer_bitwise(&xor, "UInt64", AggregateFunction::GroupBitXor).unwrap(),
        b"groupBitXor(bits)\n6\n"
    );
    let or = [
        shard("groupBitOr(bits)\n-8\n"),
        shard("groupBitOr(bits)\n3\n"),
    ];
    assert_eq!(
        merge_integer_bitwise(&or, "Int64", AggregateFunction::GroupBitOr).unwrap(),
        b"groupBitOr(bits)\n-5\n"
    );
    let and = [
        shard("groupBitAnd(bits)\n7\n"),
        shard("groupBitAnd(bits)\n3\n"),
    ];
    assert_eq!(
        merge_integer_bitwise(&and, "UInt64", AggregateFunction::GroupBitAnd).unwrap(),
        b"groupBitAnd(bits)\n3\n"
    );
}

#[test]
fn ignores_empty_nullable_shards_without_inventing_an_identity() {
    let values = [
        shard("groupBitAnd(bits)\nNULL\n"),
        shard("groupBitAnd(bits)\n7\n"),
    ];
    assert_eq!(
        merge_integer_bitwise(&values, "Nullable(UInt64)", AggregateFunction::GroupBitAnd)
            .unwrap(),
        b"groupBitAnd(bits)\n7\n"
    );
    assert_eq!(
        merge_integer_bitwise(
            &values[..1],
            "Nullable(UInt64)",
            AggregateFunction::GroupBitAnd
        )
        .unwrap(),
        b"groupBitAnd(bits)\nNULL\n"
    );
}

#[test]
fn rejects_corrupt_and_incompatible_shard_states() {
    for bad in [
        "groupBitXor(bits)\n",
        "groupBitXor(bits)\n4\n5\n",
        "groupBitXor(bits)\n-1\n",
        "groupBitXor(bits)\n2.5\n",
    ] {
        assert_eq!(
            merge_integer_bitwise(&[shard(bad)], "UInt64", AggregateFunction::GroupBitXor),
            Err(MergeError::InvalidBitwise),
        );
    }
    assert_eq!(
        merge_integer_bitwise(
            &[
                shard("groupBitAnd(bits)\n1\n"),
                shard("groupBitAnd(other)\n2\n")
            ],
            "UInt64",
            AggregateFunction::GroupBitAnd
        ),
        Err(MergeError::SchemaMismatch),
    );
}

#[test]
fn policy_rejects_pagination() {
    for function in ["groupBitAnd", "groupBitOr", "groupBitXor"] {
        let sql = format!("SELECT {function}(bits) FROM events WHERE id > 1");
        assert!(is_safe_bitwise(&parse_select(&sql).unwrap()));
        let sql = format!("SELECT {function}(bits) FROM events LIMIT 1");
        assert!(!is_safe_bitwise(&parse_select(&sql).unwrap()));
    }
}
