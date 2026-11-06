use super::*;

#[test]
fn batch_ranking_matches_per_shard_placement_for_all_replica_counts() {
    let nodes = (0..32).map(|i| format!("node_{i:02}")).collect::<Vec<_>>();
    let shard_names = (0..256)
        .map(|i| format!("tenant_{i:04}"))
        .collect::<Vec<_>>();
    let shards = shard_names.iter().map(String::as_str).collect::<Vec<_>>();
    for replicas in [1, 2, 3, 7, 16, 32] {
        let batch = replicas_for_many(&shards, &nodes, replicas).unwrap();
        assert_eq!(batch.len(), shards.len());
        for (shard, actual) in shards.iter().zip(batch) {
            assert_eq!(
                actual,
                replicas_for(shard, &nodes, replicas).unwrap(),
                "{shard}"
            );
        }
    }
}

#[test]
fn bad_node_sets_and_invalid_shards_fail_atomically() {
    let nodes = vec!["n1".to_owned(), "n2".to_owned(), "n3".to_owned()];
    assert_eq!(
        replicas_for_many(&[], &nodes, 2).unwrap(),
        Vec::<Vec<String>>::new()
    );
    assert!(replicas_for_many(&["good", "../bad"], &nodes, 2).is_err());
    assert!(
        replicas_for_many(&["good"], &["duplicate".into(), "duplicate".into()], 1).is_err()
    );
    assert!(replicas_for_many(&["good"], &nodes, 4).is_err());
    assert!(replicas_for_many(&vec!["shard"; 4097], &nodes, 1).is_err());
}
