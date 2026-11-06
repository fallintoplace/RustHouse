use super::*;

#[test]
fn uppercase_and_lowercase_wire_hex_are_the_same_uuid() {
    let one =
        b"id\n550e8400-e29b-41d4-a716-446655440000\n00000000-0000-0000-0000-000000000000\n"
            .to_vec();
    let two =
        b"id\n550E8400-E29B-41D4-A716-446655440000\n00000000-0000-0000-0000-000000000000\n"
            .to_vec();
    assert_eq!(
        merge_distinct_uuid(&[(10, one), (10, two)], "id", false).unwrap(),
        b"count(DISTINCT id)\n2\n"
    );
}

#[test]
fn supports_nullable_uuid_and_rejects_null_in_required_columns() {
    let shard = [(
        10,
        b"id\n\\N\n550e8400-e29b-41d4-a716-446655440000\n".to_vec(),
    )];
    assert_eq!(
        merge_distinct_uuid(&shard, "id", true).unwrap(),
        b"count(DISTINCT id)\n1\n"
    );
    assert_eq!(
        merge_distinct_uuid(&shard, "id", false),
        Err(MergeError::InvalidDistinctCount)
    );
}

#[test]
fn malformed_uuid_wire_never_enters_global_distinct_keys() {
    for value in [
        "550e8400e29b41d4a716446655440000",
        "550e8400-e29b-41d4-a716-44665544000Z",
        "550e8400_e29b-41d4-a716-446655440000",
        "550e8400-e29b-41d4-a716-446655440000 ",
        "",
    ] {
        let body = format!("id\n{value}\n").into_bytes();
        assert_eq!(
            merge_distinct_uuid(&[(10, body)], "id", false),
            Err(MergeError::InvalidDistinctCount),
            "{value:?}"
        );
    }
}

#[test]
fn validates_shard_cap_before_deduplicating_uuid_rows() {
    let key = "550e8400-e29b-41d4-a716-446655440000";
    let wire = format!("id\n{key}\n{key}\n").into_bytes();
    assert_eq!(
        merge_distinct_uuid(&[(2, wire.clone())], "id", false),
        Err(MergeError::ShardRowLimit)
    );
    assert_eq!(
        merge_distinct_uuid(&[(5, wire)], "id", false).unwrap(),
        b"count(DISTINCT id)\n1\n"
    );
}
