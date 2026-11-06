use super::*;

#[test]
fn global_grouped_sum_pages_after_merging_more_groups_than_output_cap() {
    let query = parse_select(
        "SELECT region, sum(amount) FROM events GROUP BY region ORDER BY 2 DESC LIMIT 1",
    )
    .unwrap();
    let shards = [
        (8, b"region\tsum(amount)\na\t2\nb\t3\n".to_vec()),
        (8, b"region\tsum(amount)\nc\t4\nd\t7\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_sums_query(&shards, 1, "Int64", "region", "amount", Some(&query))
            .unwrap(),
        b"region\tsum(amount)\nd\t7\n"
    );
    let no_limit =
        parse_select("SELECT region, sum(amount) FROM events GROUP BY region").unwrap();
    assert_eq!(
        merge_grouped_integer_sums_query(
            &shards,
            1,
            "Int64",
            "region",
            "amount",
            Some(&no_limit)
        ),
        Err(MergeError::CoordinatorLimit)
    );
}

#[test]
fn grouped_sum_having_can_reduce_more_groups_than_final_cap() {
    let query = parse_select(
        "SELECT region, sum(amount) FROM events GROUP BY region HAVING sum(amount) > 6 LIMIT 1",
    )
    .unwrap();
    let shards = [
        (8, b"region\tsum(amount)\na\t2\nb\t3\n".to_vec()),
        (8, b"region\tsum(amount)\nc\t4\nd\t7\n".to_vec()),
    ];
    assert_eq!(
        merge_grouped_integer_sums_query(&shards, 1, "Int64", "region", "amount", Some(&query))
            .unwrap(),
        b"region\tsum(amount)\nd\t7\n"
    );
}
