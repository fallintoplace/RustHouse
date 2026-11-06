use super::*;

#[test]
fn global_union_merges_sketch_states_not_estimates() {
    let mut first = "00".repeat(1 << 14);
    first.replace_range(0..2, "01");
    let mut second = "00".repeat(1 << 14);
    second.replace_range(2..4, "01");
    let shards = [
        (100, format!("uniqState(uid)\n{first}\n").into_bytes()),
        (100, format!("uniqState(uid)\n{second}\n").into_bytes()),
    ];
    assert_eq!(
        merge_uniq_shard_states(&shards, "uid", None).unwrap(),
        b"uniq(uid)\n2\n"
    );
    assert_eq!(
        merge_uniq_shard_states(&shards, "uid", Some("active_users")).unwrap(),
        b"active_users\n2\n"
    );
}

#[test]
fn corrupted_or_mismatched_shard_sketches_reject_entire_query() {
    let cases = [
        "uniqState(uid)\n0\n".to_string(),
        "uniqState(uid)\n".to_string(),
        "wrong\n".to_string(),
        format!("uniqState(uid)\n{}\n", "ff".repeat(1 << 14)),
        format!("uniqState(uid)\n{}\n\n", "00".repeat(1 << 14)),
    ];
    for content in cases {
        assert_eq!(
            merge_uniq_shard_states(&[(100, content.into_bytes())], "uid", None),
            Err(MergeError::InvalidUniqState)
        );
    }
}

#[test]
fn query_rewrite_keeps_where_predicates_and_rejects_order() {
    let sql = "SELECT uniq(uid) FROM events WHERE region = 'FROM east'";
    let select = parse_select(sql).unwrap();
    assert!(is_safe_uniq(&select));
    assert_eq!(
        uniq_state_shard_sql(sql, &select).as_deref(),
        Some("SELECT uniqState(uid) FROM events WHERE region = 'FROM east'")
    );
    assert!(parse_select("SELECT uniq(uid) FROM events ORDER BY uid").is_err());
}
