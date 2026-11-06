    use super::*;

    #[test]
    fn resumed_timeline_matches_absolute_single_node_backoff() {
        let peers = [("a", 7u64), ("b", 100u64), ("c", u64::MAX)];
        for first in [0, 1, 5, 16, 31, 32] {
            let attempts = 33 - first;
            let timeline = plan_replica_retry_timeline_from(
                &peers,
                first,
                attempts,
                10,
                500,
                Duration::from_secs(60),
            )
            .unwrap();
            for &(node, seed) in &peers {
                let per_node = timeline
                    .iter()
                    .filter(|event| event.node == node)
                    .collect::<Vec<_>>();
                let mut elapsed = Duration::ZERO;
                for (offset, event) in per_node.iter().enumerate() {
                    let attempt = first + offset as u32;
                    elapsed += retry_delay(attempt, 10, 500, seed).unwrap();
                    assert_eq!(event.attempt, attempt);
                    assert_eq!(event.due_after, elapsed);
                }
                assert_eq!(per_node.len(), attempts as usize);
            }
            assert!(timeline.windows(2).all(|pair| {
                (pair[0].due_after, pair[0].node.as_str(), pair[0].attempt)
                    <= (pair[1].due_after, pair[1].node.as_str(), pair[1].attempt)
            }));
        }
    }

    #[test]
    fn default_schedule_and_resumed_zero_epoch_are_identical() {
        let peers = [("west", 1), ("east", 42)];
        let deadline = Duration::from_secs(4);
        assert_eq!(
            plan_replica_retry_timeline(&peers, 33, 25, 400, deadline),
            plan_replica_retry_timeline_from(&peers, 0, 33, 25, 400, deadline)
        );
        assert!(
            plan_replica_retry_timeline_from(&peers, 32, 2, 1, 5, Duration::from_secs(1)).is_err()
        );
        assert!(plan_replica_retry_timeline_from(&peers, 34, 0, 1, 5, Duration::ZERO).is_err());
        assert!(
            plan_replica_retry_timeline_from(&peers, 33, 0, 1, 5, Duration::ZERO)
                .unwrap()
                .is_empty()
        );
    }
