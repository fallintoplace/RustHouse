use super::*;

#[test]
fn paginates_global_count_ranking_with_stable_ties() {
    let shards = vec![
        (250_000, b"dim\tcount\nA\t1\nB\t2\nC\t1\nD\t1\n".to_vec()),
        (250_000, b"dim\tcount\nA\t4\nB\t1\nC\t2\nD\t1\n".to_vec()),
    ];
    let sql = parse_select(
        "SELECT dim, count(*) FROM facts GROUP BY dim ORDER BY count DESC LIMIT 2 OFFSET 1",
    )
    .unwrap();
    let merged = merge_grouped_counts_query(&shards, 2, Some(&sql)).unwrap();
    assert_eq!(merged, b"dim\tcount\nB\t3\nC\t3\n");
}

#[test]
fn zero_length_ranked_page_keeps_only_header() {
    let shards = vec![(250_000, b"dim\tcount\nA\t3\nB\t2\n".to_vec())];
    let sql = parse_select(
        "SELECT dim, count(*) FROM facts GROUP BY dim ORDER BY count DESC LIMIT 0",
    )
    .unwrap();
    let merged = merge_grouped_counts_query(&shards, 1, Some(&sql)).unwrap();
    assert_eq!(merged, b"dim\tcount\n");
}
