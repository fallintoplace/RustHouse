use super::*;

#[test]
fn heap_merge_matches_global_sort_for_full_and_partial_schedules() {
    let owned = (0..64u64)
        .map(|index| (format!("node_{index:02}"), index.wrapping_mul(1_000_003)))
        .collect::<Vec<_>>();
    let mut nodes = owned
        .iter()
        .map(|(name, seed)| (name.as_str(), *seed))
        .collect::<Vec<_>>();

    for deadline in [
        Duration::ZERO,
        Duration::from_millis(1),
        Duration::from_millis(300),
        Duration::from_secs(15),
        Duration::MAX,
    ] {
        for attempts in [0, 1, 12, 33] {
            let actual =
                plan_replica_retry_timeline(&nodes, attempts, 100, 1_000, deadline).unwrap();
            let mut expected = Vec::new();
            for &(node, seed) in &nodes {
                let mut due_after = Duration::ZERO;
                for (attempt, delay) in plan_retry_delays(attempts, 100, 1_000, seed, deadline)
                    .unwrap()
                    .into_iter()
                    .enumerate()
                {
                    due_after += delay;
                    expected.push(ReplicaRetryEvent {
                        node: node.to_owned(),
                        attempt: attempt as u32,
                        due_after,
                    });
                }
            }
            expected.sort_unstable_by(|a, b| {
                a.due_after
                    .cmp(&b.due_after)
                    .then_with(|| a.node.cmp(&b.node))
                    .then_with(|| a.attempt.cmp(&b.attempt))
            });
            assert_eq!(
                actual, expected,
                "attempts={attempts}, deadline={deadline:?}"
            );
            nodes.reverse();
            assert_eq!(
                plan_replica_retry_timeline(&nodes, attempts, 100, 1_000, deadline).unwrap(),
                expected
            );
            nodes.reverse();
        }
    }
}
