use super::*;

#[test]
fn membership_order_does_not_change_placement() {
    let nodes = vec!["node_a".into(), "node_b".into(), "node_c".into()];
    let reversed = nodes.iter().rev().cloned().collect::<Vec<_>>();
    let expected = replicas_for("events-2026", &nodes, 2).unwrap();
    assert_eq!(expected, replicas_for("events-2026", &reversed, 2).unwrap());
    assert_eq!(expected.len(), 2);
    assert_ne!(expected[0], expected[1]);
}

#[test]
fn single_replica_path_matches_full_ranking_for_many_shards() {
    let nodes = vec!["node_c".into(), "node_a".into(), "node_b".into()];
    let weighted = [("node_c", 8u16), ("node_a", 2), ("node_b", 1)];
    for index in 0..128 {
        let shard = format!("shard_{index}");
        let best = replicas_for(&shard, &nodes, 1).unwrap();
        let ranked = replicas_for(&shard, &nodes, nodes.len()).unwrap();
        assert_eq!(best[0], ranked[0]);
        let best = weighted_replicas_for(&shard, &weighted, 1).unwrap();
        let ranked = weighted_replicas_for(&shard, &weighted, weighted.len()).unwrap();
        assert_eq!(best[0], ranked[0]);
    }
}

#[test]
fn weighted_placement_is_stable_and_prefers_more_capacity() {
    let nodes = [("small", 1u16), ("large", 16u16)];
    assert_eq!(
        weighted_replicas_for("example", &nodes, 2).unwrap(),
        weighted_replicas_for(
            "example",
            &nodes.iter().rev().copied().collect::<Vec<_>>(),
            2
        )
        .unwrap()
    );
    let selected = (0..256)
        .filter(|i| {
            weighted_replicas_for(&format!("shard_{i}"), &nodes, 1).unwrap()[0] == "large"
        })
        .count();
    assert!(
        selected > 128,
        "weighted placement did not favor larger node: {selected}"
    );
    assert!(weighted_replicas_for("s", &[("n", 0)], 1).is_err());
    assert!(weighted_replicas_for("s", &[("n", 1025)], 1).is_err());
    assert!(weighted_replicas_for("s", &[("n", 1), ("n", 2)], 1).is_err());
    assert!(weighted_replicas_for("s", &nodes, 3).is_err());
}

#[test]
fn rejects_unsafe_shard_names() {
    let nodes = ["node_a".to_owned()];
    for shard in [
        "../private",
        "one/two",
        ".hidden",
        "a..b",
        "a\\b",
        "é",
        "x\n",
    ] {
        assert!(replicas_for(shard, &nodes, 1).is_err(), "{shard}");
        assert!(
            weighted_replicas_for(shard, &[("node_a", 1)], 1).is_err(),
            "{shard}"
        );
    }
    assert!(replicas_for("events.2026", &nodes, 1).is_ok());
}

#[test]
fn rejects_duplicate_nodes_and_bad_replica_counts() {
    assert!(replicas_for("shard", &["node".into(), "node".into()], 1).is_err());
    assert!(replicas_for("shard", &["node".into()], 2).is_err());
    assert!(replicas_for("shard", &["../bad".into()], 1).is_err());
}
#[test]
fn increasing_one_node_weight_cannot_demote_its_primary_placement() {
    let baseline = [("a", 1u16), ("b", 1u16), ("c", 1u16)];
    let heavier = [("a", 64u16), ("b", 1u16), ("c", 1u16)];
    for shard in 0..512 {
        let shard = format!("events_{shard}");
        let original = weighted_replicas_for(&shard, &baseline, 1).unwrap();
        let updated = weighted_replicas_for(&shard, &heavier, 1).unwrap();
        if original[0] == "a" {
            assert_eq!(updated[0], "a", "{shard}");
        }
    }
}

#[test]
fn weighted_primary_matches_full_ranking_on_many_shards() {
    let nodes = (0..64)
        .map(|index| (format!("node_{index}"), 1 + index as u16))
        .collect::<Vec<_>>();
    let weighted = nodes
        .iter()
        .map(|(node, weight)| (node.as_str(), *weight))
        .collect::<Vec<_>>();
    for index in 0..32 {
        let shard = format!("weighted_{index}");
        let first = weighted_replicas_for(&shard, &weighted, 1).unwrap();
        let all = weighted_replicas_for(&shard, &weighted, weighted.len()).unwrap();
        assert_eq!(first[0], all[0]);
    }
}

#[test]
fn one_pass_primary_matches_full_rendezvous_ranking() {
    let nodes = (0..64)
        .map(|index| format!("node_{index}"))
        .collect::<Vec<_>>();
    for index in 0..64 {
        let shard = format!("shard_{index}");
        let first = replicas_for(&shard, &nodes, 1).unwrap();
        let all = replicas_for(&shard, &nodes, nodes.len()).unwrap();
        assert_eq!(first[0], all[0]);
    }
}
