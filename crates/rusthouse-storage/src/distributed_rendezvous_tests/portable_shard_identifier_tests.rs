use super::*;

#[test]
fn rejects_trailing_dot_shards_for_replicated_object_paths() {
    let nodes = vec!["node_a".to_owned(), "node_b".to_owned()];
    for shard in ["part.", "events.2026.", "a..b"] {
        assert!(replicas_for(shard, &nodes, 1).is_err(), "{shard}");
        assert!(
            weighted_replicas_for(shard, &[("node_a", 1)], 1).is_err(),
            "{shard}"
        );
    }
    for shard in ["events.2026", "a_1", "x"] {
        assert!(replicas_for(shard, &nodes, 1).is_ok(), "{shard}");
        assert!(
            crate::distributed_object_key::ObjectKey::new(
                "cluster", shard, 1, "part", "data.bin"
            )
            .is_ok()
        );
    }
}
