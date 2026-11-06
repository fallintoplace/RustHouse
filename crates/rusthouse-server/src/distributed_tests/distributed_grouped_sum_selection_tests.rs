use super::*;

#[test]
fn partial_rank_matches_stable_global_sum_sort_at_scale() {
    let mut first = b"dimension\tsum(metric)\n".to_vec();
    let mut second = b"dimension\tsum(metric)\n".to_vec();
    let mut canonical = Vec::new();
    for shard in 0..2 {
        for i in (shard..3000).step_by(2) {
            let value = (i * 379 % 997) as u64;
            let row = format!("g{i:04}\t{value}\n");
            if shard == 0 {
                first.extend_from_slice(row.as_bytes());
            } else {
                second.extend_from_slice(row.as_bytes());
            }
            canonical.push((i, value));
        }
    }
    let shards = [(4000usize, first), (4000usize, second)];
    for descending in [true, false] {
        let mut order = canonical.clone();
        if descending {
            order.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        } else {
            order.sort_by_key(|entry| entry.1);
        }
        for offset in [0, 1, 25, 2999, 3000] {
            for limit in [0, 1, 7, 100, 3000] {
                let direction = if descending { "DESC" } else { "ASC" };
                let sql = format!(
                    "SELECT dimension, sum(metric) FROM events GROUP BY dimension ORDER BY 2 {direction} LIMIT {limit} OFFSET {offset}"
                );
                let select = rusthouse_query::parse_select(&sql).unwrap();
                let actual = merge_grouped_integer_sums_query(
                    &shards,
                    4000,
                    "UInt64",
                    "dimension",
                    "metric",
                    Some(&select),
                )
                .unwrap();
                let mut expected = "dimension\tsum(metric)\n".to_string();
                for (i, amount) in order.iter().skip(offset).take(limit) {
                    expected.push_str(&format!("g{i:04}\t{amount}\n"));
                }
                assert_eq!(actual, expected.as_bytes(), "{sql}");
            }
        }
    }
}
