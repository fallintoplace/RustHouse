use super::*;

#[test]
fn stack_winners_match_full_weighted_sort_for_many_shards_and_memberships() {
    let names = [
        "node_a", "node_b", "node_c", "node_d", "node_e", "node_f", "node_g", "node_h",
    ];
    let weights = [1u16, 2, 7, 31, 127, 509, 1000, 1024];
    for count in 2..=names.len() {
        let nodes = names
            .iter()
            .zip(weights)
            .take(count)
            .map(|(&name, weight)| (name, weight))
            .collect::<Vec<_>>();
        for id in 0..256 {
            let shard = format!("shard_{id}");
            let seed = shard_seed(&shard);
            let mut ranked = nodes
                .iter()
                .map(|&(name, weight)| {
                    let score = score_with_seed(seed, name);
                    (
                        -hash_to_open_unit_interval(score).ln() / f64::from(weight),
                        name,
                    )
                })
                .collect::<Vec<_>>();
            ranked.sort_unstable_by(weighted_rank_order);
            for replicas in [1, 2] {
                let expected = ranked
                    .iter()
                    .take(replicas)
                    .map(|&(_, name)| name.to_owned())
                    .collect::<Vec<_>>();
                assert_eq!(
                    weighted_replicas_for(&shard, &nodes, replicas).unwrap(),
                    expected
                );
            }
        }
    }
}

#[test]
fn still_validates_late_invalid_nodes_before_returning_primary() {
    for count in [1, 2] {
        let invalid = [("node_a", 2), ("node_b", 0)];
        assert!(weighted_replicas_for("shard_1", &invalid, count).is_err());
        let duplicate = [("node_a", 2), ("node_a", 3)];
        assert!(weighted_replicas_for("shard_1", &duplicate, count).is_err());
    }
}
