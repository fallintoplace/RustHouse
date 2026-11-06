use super::*;

#[test]
fn distributed_bitmap32_merges_high_cardinality_exactly() {
    let mut body = String::from("id\n");
    for id in 0..125_000u32 {
        body.push_str(&format!("{id}\n"));
    }
    let shards = [shard(250_000, &body), shard(250_000, &body)];
    assert_eq!(
        merge_distinct_bitmap32(&shards, "id", false).unwrap(),
        b"count(DISTINCT id)\n125000\n"
    );
    assert_eq!(
        merge_distinct_bitmap32(&[shard(10, "id\n-1\n0\n-1\n\\N\n")], "id", true).unwrap(),
        b"count(DISTINCT id)\n2\n"
    );
    assert_eq!(
        validated_schema_kind(br#"[{"name":"id","type":"UInt32"}]"#, "id"),
        Some("UInt32")
    );
}

#[test]
fn extracts_column_types_from_one_validated_schema_response() {
    let response = remote::PeerResponse {
        status: 200,
        content_type: "application/json; charset=utf-8".into(),
        body: b"{\"rows\":12,\"columns\":[{\"name\":\"region\",\"type\":\"String\"},{\"name\":\"amount\",\"type\":\"Int64\"}]}\n".to_vec(),
    };
    assert!(schema_signature(&response).is_ok());
    assert_eq!(shard_column_kind(&response, "region").unwrap(), "String");
    assert_eq!(shard_integer_kind(&response, "amount").unwrap(), "Int64");
    assert!(shard_integer_kind(&response, "region").is_err());
}

fn shard(cap: usize, data: &str) -> (usize, Vec<u8>) {
    (cap, data.as_bytes().to_vec())
}

#[test]
fn globally_ranks_grouped_counts_after_reducing_shards() {
    let sql = "SELECT region, count(*) FROM events GROUP BY region HAVING count(*) >= 3 ORDER BY count DESC LIMIT 2 OFFSET 1";
    let select = parse_select(sql).unwrap();
    assert!(is_safe_grouped_count(&select));
    assert_eq!(
        grouped_count_shard_sql(sql, &select).as_deref(),
        Some("SELECT region, count(*) FROM events GROUP BY region")
    );
    let shards = [
        shard(10, "region\tcount\nwest\t2\neast\t5\nsouth\t1\n"),
        shard(10, "region\tcount\neast\t1\nwest\t4\nnorth\t9\n"),
    ];
    // Global counts: north=9, west=6, east=6, south=1.
    // HAVING excludes south, stable ties keep west ahead of east,
    // OFFSET discards north and LIMIT returns the two remaining groups.
    assert_eq!(
        merge_grouped_counts_query(&shards, 2, Some(&select)).unwrap(),
        b"region\tcount\nwest\t6\neast\t6\n"
    );
    // Never push a local top-k to shards: it may remove a global winner.
    assert!(!is_safe_grouped_count(
        &parse_select(
            "SELECT region, count(*) FROM events GROUP BY region ORDER BY region LIMIT 2"
        )
        .unwrap()
    ));
}

#[test]
fn grouped_global_having_rejects_capped_and_malformed_shards() {
    let query = parse_select(
        "SELECT region, count(*) FROM events GROUP BY region HAVING count(*) > 0 LIMIT 1",
    )
    .unwrap();
    assert_eq!(
        merge_grouped_counts_query(&[shard(2, "region\tcount\na\t1\nb\t2\n")], 1, Some(&query)),
        Err(MergeError::ShardRowLimit)
    );
    assert_eq!(
        merge_grouped_counts_query(
            &[shard(10, "region\tcount\na\tnot-a-number\n")],
            1,
            Some(&query)
        ),
        Err(MergeError::InvalidGroupedCount)
    );
}

#[test]
fn global_grouped_sum_having_and_order_apply_after_reduction() {
    let sql = "SELECT region, sum(amount) FROM events GROUP BY region HAVING sum(amount) > 100 ORDER BY 2 DESC LIMIT 2";
    let select = parse_select(sql).unwrap();
    assert!(is_safe_grouped_sum(&select));
    assert_eq!(
        grouped_count_shard_sql(sql, &select).as_deref(),
        Some("SELECT region, sum(amount) FROM events GROUP BY region")
    );
    let shards = [
        shard(10, "region\tsum(amount)\nwest\t80\neast\t90\n"),
        shard(10, "region\tsum(amount)\nwest\t70\neast\t15\nnorth\t151\n"),
    ];
    assert_eq!(
        merge_grouped_integer_sums_query(
            &shards,
            10,
            "Int64",
            "region",
            "amount",
            Some(&select)
        )
        .unwrap(),
        b"region\tsum(amount)\nnorth\t151\nwest\t150\n"
    );
}

#[test]
fn distributed_count_distinct_uses_global_unique_values() {
    let query = "SELECT count(DISTINCT category) FROM events WHERE id >= 5";
    let parsed = parse_select(query).unwrap();
    assert!(is_safe_count_distinct(&parsed));
    assert_eq!(
        distinct_shard_sql(query, &parsed).as_deref(),
        Some("SELECT DISTINCT category FROM events WHERE id >= 5")
    );
    assert!(!is_safe_count_distinct(
        &parse_select("SELECT count(DISTINCT category) FROM events LIMIT 1").unwrap()
    ));
    let shards = [
        shard(10, "category\nred\nblue\n\\N\n"),
        shard(10, "category\nblue\ngreen\n"),
        shard(10, "category\nred\n"),
    ];
    assert_eq!(
        merge_distinct_counts(&shards, "category").unwrap(),
        b"count(DISTINCT category)\n3\n"
    );
    assert_eq!(
        merge_distinct_counts(&[shard(2, "category\na\nb\n")], "category"),
        Err(MergeError::ShardRowLimit)
    );
    assert_eq!(
        merge_distinct_counts(&[shard(10, "other\na\n")], "category"),
        Err(MergeError::InvalidDistinctCount)
    );
    assert_eq!(
        merge_distinct_counts(&[shard(10, "category\na")], "category"),
        Err(MergeError::InvalidDistinctCount)
    );
}

#[test]
fn merges_rows_in_peer_order_without_duplicate_headers() {
    let shards = [
        shard(10, "id\tname\n1\tAda\n2\tLin\n"),
        shard(10, "id\tname\n"),
        shard(10, "id\tname\n3\tGrace\n"),
    ];
    assert_eq!(
        merge_tsv(&shards, 3).unwrap(),
        b"id\tname\n1\tAda\n2\tLin\n3\tGrace\n"
    );
}

#[test]
fn applies_global_limit_and_offset_in_peer_order() {
    let shards = [shard(8, "id\n1\n2\n3\n"), shard(8, "id\n4\n5\n")];
    assert_eq!(
        merge_tsv_page(&shards, 2, 2, Some(2)).unwrap(),
        b"id\n3\n4\n"
    );
    assert_eq!(merge_tsv_page(&shards, 2, 3, None).unwrap(), b"id\n4\n5\n");
    assert_eq!(merge_tsv_page(&shards, 2, 0, Some(0)).unwrap(), b"id\n");
    assert_eq!(merge_tsv_page(&shards, 2, 999, Some(2)).unwrap(), b"id\n");
    assert_eq!(
        merge_tsv_page(&shards, 1, 0, Some(2)),
        Err(MergeError::CoordinatorLimit)
    );
    assert_eq!(
        merge_tsv_page(&[shard(2, "id\n1\n2\n")], 10, 0, Some(1)),
        Err(MergeError::ShardRowLimit)
    );
}

#[test]
fn strips_only_validated_global_pagination_from_shard_sql() {
    for sql in [
        "SELECT id FROM events LIMIT 2",
        "SELECT id FROM events LIMIT 2 OFFSET 3",
    ] {
        let parsed = parse_select(sql).unwrap();
        assert!(is_safe_scan(&parsed), "{sql}");
        assert_eq!(
            unpaged_scan_sql(sql, &parsed),
            Some("SELECT id FROM events".to_string()),
            "{sql}"
        );
    }
    let sql = "SELECT name FROM events WHERE name = 'LIMIT 2' LIMIT 1";
    let parsed = parse_select(sql).unwrap();
    assert_eq!(
        unpaged_scan_sql(sql, &parsed),
        Some("SELECT name FROM events WHERE name = 'LIMIT 2'".into())
    );
}

#[test]
fn refuses_incomplete_or_incompatible_shard_outputs() {
    assert_eq!(
        merge_tsv(&[shard(10, "id\n1\n"), shard(10, "name\n2\n")], 5),
        Err(MergeError::SchemaMismatch)
    );
    assert_eq!(
        merge_tsv(&[shard(1, "id\n1\n")], 10),
        Err(MergeError::ShardRowLimit)
    );
    assert_eq!(
        merge_tsv(&[shard(10, "id\n1\n2\n")], 1),
        Err(MergeError::CoordinatorLimit)
    );
    assert_eq!(
        merge_tsv(&[shard(10, "id\n1")], 10),
        Err(MergeError::InvalidResponse)
    );
}

#[test]
fn rejects_non_decomposable_sql() {
    for sql in [
        "SELECT count(*) FROM events",
        "SELECT id FROM events ORDER BY id",
        "SELECT DISTINCT id FROM events",
        "SELECT number FROM numbers(10)",
    ] {
        assert!(
            parse_select(sql).map_or(true, |select| !is_safe_scan(&select)),
            "{sql}"
        );
    }
    assert!(is_safe_scan(
        &parse_select("SELECT id FROM events WHERE id > 2").unwrap()
    ));
}

#[test]
fn compares_shard_column_types_independently_of_row_counts() {
    let response = |body: &'static [u8]| remote::PeerResponse {
        status: 200,
        content_type: "application/json; charset=utf-8".into(),
        body: body.to_vec(),
    };
    let a = schema_signature(&response(
        b"{\"rows\":4,\"columns\":[{\"name\":\"id\",\"type\":\"UInt64\"}]}\n",
    ))
    .unwrap();
    let b = schema_signature(&response(
        b"{\"rows\":900,\"columns\":[{\"name\":\"id\",\"type\":\"UInt64\"}]}\n",
    ))
    .unwrap();
    let different = schema_signature(&response(
        b"{\"rows\":4,\"columns\":[{\"name\":\"id\",\"type\":\"String\"}]}\n",
    ))
    .unwrap();
    assert_eq!(a, b);
    assert_ne!(a, different);
    assert!(schema_signature(&response(b"bad")).is_err());
}

#[test]
fn sums_counts_and_nullable_column_counts_safely() {
    assert!(is_safe_count(
        &parse_select("SELECT count(*) FROM events").unwrap()
    ));
    assert!(is_safe_count(
        &parse_select("SELECT count(name) FROM events WHERE id > 10").unwrap()
    ));
    assert!(!is_safe_count(
        &parse_select("SELECT count(DISTINCT name) FROM events").unwrap()
    ));
    assert_eq!(
        sum_shard_counts(&[
            shard(10, "count\n20\n"),
            shard(10, "count\n0\n"),
            shard(10, "count\n300\n"),
        ])
        .unwrap(),
        b"count\n320\n"
    );
}

#[test]
fn reduces_count_if_for_disjoint_shards() {
    assert!(is_safe_count_if(
        &parse_select("SELECT countIf(flag) FROM events WHERE id > 0").unwrap()
    ));
    for sql in [
        "SELECT countIf(flag) FROM events LIMIT 1",
        "SELECT countIf(flag) FROM numbers(10)",
        "SELECT countIf(flag) FROM events OFFSET 1",
        "SELECT sum(flag) FROM events",
    ] {
        assert!(
            parse_select(sql).map_or(true, |query| !is_safe_count_if(&query)),
            "{sql}"
        );
    }
    assert_eq!(
        sum_shard_counts(&[
            shard(1, "countIf(flag)\n2\n"),
            shard(1, "countIf(flag)\n0\n"),
            shard(1, "countIf(flag)\n3\n"),
        ])
        .unwrap(),
        b"countIf(flag)\n5\n"
    );
    assert_eq!(
        sum_shard_counts(&[
            shard(1, "countIf(flag)\n1\n"),
            shard(1, "countIf(other)\n2\n"),
        ]),
        Err(MergeError::SchemaMismatch)
    );
}

#[test]
fn refuses_invalid_count_responses_and_overflow() {
    assert_eq!(
        sum_shard_counts(&[shard(10, "count\n2\n"), shard(10, "sum\n3\n")]),
        Err(MergeError::SchemaMismatch)
    );
    for invalid in ["count\n-2\n", "count\nNaN\n", "count\n3\n4\n", "count\n3"] {
        assert_eq!(
            sum_shard_counts(&[shard(10, invalid)]),
            Err(MergeError::InvalidCount)
        );
    }
    assert_eq!(
        sum_shard_counts(&[
            shard(10, "count\n18446744073709551615\n"),
            shard(10, "count\n1\n"),
        ]),
        Err(MergeError::CountOverflow)
    );
}

#[test]
fn reduces_grouped_count_across_disjoint_shards() {
    assert!(is_safe_grouped_count(
        &parse_select("SELECT kind, count(*) FROM events GROUP BY kind").unwrap()
    ));
    assert!(is_safe_grouped_count(
        &parse_select("SELECT kind, count(*) FROM events WHERE id >= 1 GROUP BY kind").unwrap()
    ));
    let shards = [
        shard(10, "kind\tcount\nalpha\t2\nbeta\t3\n"),
        shard(10, "kind\tcount\nbeta\t4\n\t1\n"),
        shard(10, "kind\tcount\nalpha\t5\n"),
    ];
    assert_eq!(
        merge_grouped_counts(&shards, 3).unwrap(),
        b"kind\tcount\nalpha\t7\nbeta\t7\n\t1\n"
    );
    assert_eq!(
        merge_grouped_counts(&[shard(10, "kind\tcount\n")], 10).unwrap(),
        b"kind\tcount\n"
    );
}

#[test]
fn rejects_unsafe_grouped_count_and_incomplete_shard_output() {
    assert!(is_safe_grouped_count(
        &parse_select("SELECT kind, count(*) FROM events GROUP BY kind LIMIT 1").unwrap()
    ));
    for sql in [
        "SELECT kind, count(*) FROM events GROUP BY kind ORDER BY kind",
        "SELECT kind, count(*) FROM events GROUP BY kind HAVING count > 1",
        "SELECT id, sum(value) FROM events GROUP BY id",
        "SELECT kind, count(*) FROM numbers(10) GROUP BY kind",
    ] {
        assert!(
            parse_select(sql).map_or(true, |select| !is_safe_grouped_count(&select)),
            "{sql}"
        );
    }
    assert_eq!(
        merge_grouped_counts(&[shard(10, "kind\tcount\na\t2")], 10),
        Err(MergeError::InvalidGroupedCount)
    );
    assert_eq!(
        merge_grouped_counts(&[shard(10, "kind\tcount\na\t2\t3\n")], 10),
        Err(MergeError::InvalidGroupedCount)
    );
    assert_eq!(
        merge_grouped_counts(&[shard(10, "kind\tcount\na\t-1\n")], 10),
        Err(MergeError::InvalidGroupedCount)
    );
    assert_eq!(
        merge_grouped_counts(
            &[
                shard(10, "kind\tcount\na\t1\n"),
                shard(10, "id\tcount\na\t2\n")
            ],
            10
        ),
        Err(MergeError::SchemaMismatch)
    );
    assert_eq!(
        merge_grouped_counts(&[shard(1, "kind\tcount\na\t1\n")], 10),
        Err(MergeError::ShardRowLimit)
    );
    assert_eq!(
        merge_grouped_counts(&[shard(10, "kind\tcount\na\t1\nb\t2\n")], 1),
        Err(MergeError::CoordinatorLimit)
    );
    assert_eq!(
        merge_grouped_counts(
            &[
                shard(10, "kind\tcount\na\t18446744073709551615\n"),
                shard(10, "kind\tcount\na\t1\n"),
            ],
            10
        ),
        Err(MergeError::CountOverflow)
    );
}

#[test]
fn allows_distributed_integer_min_and_max_only_without_global_clauses() {
    for sql in [
        "SELECT min(amount) FROM events WHERE id > 3",
        "SELECT max(amount) FROM events",
    ] {
        assert!(is_safe_extremum(&parse_select(sql).unwrap()), "{sql}");
    }
    for sql in [
        "SELECT max(amount) FROM events LIMIT 1",
        "SELECT min(amount) FROM numbers(10)",
        "SELECT min(amount) FROM events OFFSET 1",
        "SELECT avg(amount) FROM events",
    ] {
        assert!(
            parse_select(sql).map_or(true, |query| !is_safe_extremum(&query)),
            "{sql}"
        );
    }
}

#[test]
fn reduces_signed_and_unsigned_extrema_with_null_shards() {
    let shards = [
        shard(10, "min(amount)\nNULL\n"),
        shard(10, "min(amount)\n-9\n"),
        shard(10, "min(amount)\n13\n"),
    ];
    assert_eq!(
        merge_integer_extrema(&shards, "Nullable(Int64)", AggregateFunction::Min).unwrap(),
        b"min(amount)\n-9\n"
    );
    let shards = [
        shard(10, "max(amount)\n18446744073709551615\n"),
        shard(10, "max(amount)\n2\n"),
    ];
    assert_eq!(
        merge_integer_extrema(&shards, "UInt64", AggregateFunction::Max).unwrap(),
        b"max(amount)\n18446744073709551615\n"
    );
    assert_eq!(
        merge_integer_extrema(
            &[shard(10, "min(amount)\nNULL\n")],
            "Int64",
            AggregateFunction::Min,
        )
        .unwrap(),
        b"min(amount)\nNULL\n"
    );
}

#[test]
fn rejects_incompatible_or_malformed_extrema() {
    for bad in [
        "min(amount)\n",
        "min(amount)\n2.5\n",
        "min(amount)\n2\n3\n",
        "min(amount)\n2",
    ] {
        assert_eq!(
            merge_integer_extrema(&[shard(10, bad)], "Int64", AggregateFunction::Min),
            Err(MergeError::InvalidExtremum),
        );
    }
    assert_eq!(
        merge_integer_extrema(
            &[shard(10, "min(amount)\n1\n"), shard(10, "min(other)\n2\n"),],
            "Int64",
            AggregateFunction::Min,
        ),
        Err(MergeError::SchemaMismatch)
    );
    assert_eq!(
        merge_integer_extrema(
            &[shard(10, "max(amount)\n-1\n")],
            "UInt64",
            AggregateFunction::Max,
        ),
        Err(MergeError::InvalidExtremum)
    );
}

#[test]
fn accepts_grouped_integer_sum_without_global_clauses() {
    assert!(is_safe_grouped_sum(
        &parse_select("SELECT region, sum(amount) FROM events WHERE id > 0 GROUP BY region")
            .unwrap()
    ));
    assert!(is_safe_grouped_sum(
        &parse_select("SELECT region, sum(amount) FROM events GROUP BY region LIMIT 1")
            .unwrap()
    ));
    for sql in [
        "SELECT region, sum(amount) FROM events GROUP BY region ORDER BY region",
        "SELECT region, avg(amount) FROM events GROUP BY region",
        "SELECT region, sum(amount) FROM numbers(10) GROUP BY region",
    ] {
        assert!(
            parse_select(sql).map_or(true, |query| !is_safe_grouped_sum(&query)),
            "{sql}"
        );
    }
    assert!(group_key_type("String"));
    assert!(group_key_type("Nullable(UUID)"));
    assert!(group_key_type("UInt64"));
    assert!(!group_key_type("Float64"));
}

#[test]
fn merges_grouped_sum_with_overlapping_keys_and_null_values() {
    let shards = [
        shard(10, "region\tsum(amount)\nnorth\t15\nsouth\tNULL\n"),
        shard(10, "region\tsum(amount)\nnorth\t-7\nsouth\t5\n"),
        shard(10, "region\tsum(amount)\nnorth\tNULL\neast\t9\n"),
    ];
    assert_eq!(
        merge_grouped_integer_sums(&shards, 3, "Nullable(Int64)", "region", "amount").unwrap(),
        b"region\tsum(amount)\nnorth\t8\nsouth\t5\neast\t9\n"
    );
    assert_eq!(
        merge_grouped_integer_sums(
            &[shard(10, "id\tsum(amount)\n1\tNULL\n")],
            1,
            "UInt64",
            "id",
            "amount"
        )
        .unwrap(),
        b"id\tsum(amount)\n1\tNULL\n"
    );
    assert_eq!(
        merge_grouped_integer_sums(
            &[
                shard(10, "id\tsum(amount)\n1\t18446744073709551615\n"),
                shard(10, "id\tsum(amount)\n1\t2\n")
            ],
            1,
            "UInt64",
            "id",
            "amount"
        )
        .unwrap(),
        b"id\tsum(amount)\n1\t18446744073709551617\n"
    );
}

#[test]
fn rejects_malformed_capped_incompatible_or_overflowing_grouped_sums() {
    for bad in [
        "region\tsum(amount)\na\t1\t2\n",
        "region\tsum(amount)\na\t1.1\n",
        "region\tsum(amount)\na\t1",
        "region\tsum(amount)\na\t\n",
    ] {
        assert_eq!(
            merge_grouped_integer_sums(&[shard(10, bad)], 10, "Int64", "region", "amount"),
            Err(MergeError::InvalidGroupedSum)
        );
    }
    assert_eq!(
        merge_grouped_integer_sums(
            &[shard(1, "region\tsum(amount)\na\t1\n")],
            10,
            "Int64",
            "region",
            "amount"
        ),
        Err(MergeError::ShardRowLimit)
    );
    assert_eq!(
        merge_grouped_integer_sums(
            &[shard(10, "region\tsum(amount)\na\t1\nb\t2\n")],
            1,
            "Int64",
            "region",
            "amount"
        ),
        Err(MergeError::CoordinatorLimit)
    );
    assert_eq!(
        merge_grouped_integer_sums(
            &[shard(10, "other\tsum(amount)\na\t1\n")],
            10,
            "Int64",
            "region",
            "amount"
        ),
        Err(MergeError::SchemaMismatch)
    );
    assert_eq!(
        merge_grouped_integer_sums(
            &[
                shard(
                    10,
                    "region\tsum(amount)\na\t170141183460469231731687303715884105727\n"
                ),
                shard(10, "region\tsum(amount)\na\t1\n")
            ],
            10,
            "Int128",
            "region",
            "amount"
        ),
        Err(MergeError::SumOverflow)
    );
}

#[test]
fn only_accepts_decomposable_integer_sum_queries() {
    assert!(is_safe_sum(
        &parse_select("SELECT sum(amount) FROM events WHERE id >= 2").unwrap()
    ));
    for sql in [
        "SELECT sum(amount) FROM events ORDER BY amount",
        "SELECT sum(amount) FROM events LIMIT 1",
        "SELECT sum(amount) FROM numbers(10)",
        "SELECT sum(amount) FROM events GROUP BY amount",
        "SELECT avg(amount) FROM events",
    ] {
        assert!(
            parse_select(sql).map_or(true, |query| !is_safe_sum(&query)),
            "{sql}"
        );
    }
    assert!(integer_type("UInt128"));
    assert!(integer_type("Nullable(Int64)"));
    assert!(!integer_type("Float64"));
    assert!(!integer_type("Nullable(String)"));
}

#[test]
fn merges_exact_unsigned_signed_and_empty_integer_sums() {
    assert_eq!(
        merge_integer_sums(
            &[
                shard(10, "sum(amount)\n18446744073709551615\n"),
                shard(10, "sum(amount)\n2\n"),
                shard(10, "sum(amount)\nNULL\n"),
            ],
            "UInt64",
        )
        .unwrap(),
        b"sum(amount)\n18446744073709551617\n"
    );
    assert_eq!(
        merge_integer_sums(
            &[
                shard(10, "sum(amount)\n-13\n"),
                shard(10, "sum(amount)\n5\n"),
                shard(10, "sum(amount)\nNULL\n"),
            ],
            "Nullable(Int64)",
        )
        .unwrap(),
        b"sum(amount)\n-8\n"
    );
    assert_eq!(
        merge_integer_sums(&[shard(10, "sum(amount)\nNULL\n")], "Int64").unwrap(),
        b"sum(amount)\nNULL\n"
    );
}

#[test]
fn rejects_malformed_or_overflowing_distributed_sums() {
    for invalid in [
        "sum(amount)\n1.25\n",
        "sum(amount)\n1\n2\n",
        "sum(amount)\n",
        "sum(amount)\n1",
    ] {
        assert_eq!(
            merge_integer_sums(&[shard(10, invalid)], "Int64"),
            Err(MergeError::InvalidSum)
        );
    }
    assert_eq!(
        merge_integer_sums(
            &[shard(10, "sum(amount)\n1\n"), shard(10, "sum(total)\n2\n"),],
            "UInt64",
        ),
        Err(MergeError::SchemaMismatch)
    );
    assert_eq!(
        merge_integer_sums(
            &[
                shard(10, "sum(amount)\n340282366920938463463374607431768211455\n"),
                shard(10, "sum(amount)\n1\n"),
            ],
            "UInt128",
        ),
        Err(MergeError::SumOverflow)
    );
    assert_eq!(
        merge_integer_sums(
            &[
                shard(10, "sum(amount)\n170141183460469231731687303715884105727\n"),
                shard(10, "sum(amount)\n1\n"),
            ],
            "Int128",
        ),
        Err(MergeError::SumOverflow)
    );
}

#[test]
fn requires_post_and_configured_peers() {
    let cluster = Cluster::default();
    assert_eq!(
        handle("GET", b"", &cluster, 10).status,
        "405 Method Not Allowed"
    );
    assert_eq!(
        handle("POST", b"SELECT id FROM events", &cluster, 10).status,
        "400 Bad Request"
    );
}
