use super::*;

#[test]
fn pages_across_many_shards_match_stable_global_row_order() {
    let shards = (0..16)
        .map(|shard| {
            let mut body = b"value\n".to_vec();
            for row in 0..25 {
                body.extend_from_slice(format!("{}\n", shard * 25 + row).as_bytes());
            }
            (100, body)
        })
        .collect::<Vec<_>>();
    let expected_rows = (0..400).map(|row| format!("{row}\n")).collect::<Vec<_>>();
    for offset in [0, 1, 24, 25, 249, 399, 400, 900] {
        for limit in [None, Some(0), Some(1), Some(7), Some(100), Some(1000)] {
            let selected = expected_rows
                .iter()
                .skip(offset)
                .take(limit.unwrap_or(usize::MAX))
                .cloned()
                .collect::<String>();
            let expected = format!("value\n{selected}");
            assert_eq!(
                merge_tsv_page(&shards, 500, offset, limit).unwrap(),
                expected.into_bytes(),
                "offset={offset} limit={limit:?}"
            );
        }
    }
}

#[test]
fn late_shard_corruption_is_rejected_even_after_page_is_full() {
    let shards = vec![(10, b"x\na\nb\n".to_vec()), (10, b"x\nc\nd\n".to_vec())];
    assert_eq!(merge_tsv_page(&shards, 1, 2, Some(1)).unwrap(), b"x\nc\n");
    assert!(matches!(
        merge_tsv_page(&shards, 0, 0, Some(1)),
        Err(MergeError::CoordinatorLimit)
    ));
    let capped = vec![(10, b"x\na\n".to_vec()), (1, b"x\nb\n".to_vec())];
    assert!(matches!(
        merge_tsv_page(&capped, 2, 0, Some(1)),
        Err(MergeError::ShardRowLimit)
    ));
    let mismatched = vec![(10, b"x\na\n".to_vec()), (10, b"y\nb\n".to_vec())];
    assert!(matches!(
        merge_tsv_page(&mismatched, 2, 0, Some(1)),
        Err(MergeError::SchemaMismatch)
    ));
}
