    use super::*;

    fn elapsed(seed: u64, attempts: u32) -> Duration {
        (0..attempts)
            .map(|attempt| retry_delay(attempt, 100, 1000, seed).unwrap())
            .sum()
    }

    #[test]
    fn resumed_events_match_uninterrupted_absolute_schedule() {
        let nodes = [("west", 3), ("east", 99), ("south", u64::MAX)];
        let checkpoints = [
            ReplicaRetryCheckpoint {
                node: "west",
                seed: 3,
                next_attempt: 4,
                elapsed: elapsed(3, 4),
            },
            ReplicaRetryCheckpoint {
                node: "east",
                seed: 99,
                next_attempt: 7,
                elapsed: elapsed(99, 7),
            },
            ReplicaRetryCheckpoint {
                node: "south",
                seed: u64::MAX,
                next_attempt: 0,
                elapsed: Duration::ZERO,
            },
        ];
        let deadline = Duration::from_secs(10);
        let full = plan_replica_retry_timeline(&nodes, 33, 100, 1000, deadline).unwrap();
        let expected = full
            .into_iter()
            .filter(|event| {
                checkpoints.iter().any(|checkpoint| {
                    event.node == checkpoint.node && event.attempt >= checkpoint.next_attempt
                })
            })
            .collect::<Vec<_>>();
        let resumed =
            plan_replica_retry_timeline_from_checkpoints(&checkpoints, 33, 100, 1000, deadline)
                .unwrap();
        assert_eq!(resumed, expected);
    }

    #[test]
    fn rejects_tampered_and_duplicate_checkpoints() {
        let valid = ReplicaRetryCheckpoint {
            node: "west",
            seed: 3,
            next_attempt: 4,
            elapsed: elapsed(3, 4),
        };
        let changed = ReplicaRetryCheckpoint {
            elapsed: valid.elapsed + Duration::from_millis(1),
            ..valid
        };
        assert!(
            plan_replica_retry_timeline_from_checkpoints(&[changed], 33, 100, 1000, Duration::MAX)
                .is_err()
        );
        assert!(
            plan_replica_retry_timeline_from_checkpoints(&[valid], 3, 100, 1000, Duration::MAX)
                .is_err()
        );
        assert!(
            plan_replica_retry_timeline_from_checkpoints(
                &[valid, valid],
                33,
                100,
                1000,
                Duration::MAX
            )
            .is_err()
        );
        assert!(
            plan_replica_retry_timeline_from_checkpoints(&[valid], 33, 0, 1000, Duration::MAX)
                .is_err()
        );
    }

    #[test]
    fn deadline_excludes_future_events_but_preserves_checkpoint_validity() {
        let checkpoint = ReplicaRetryCheckpoint {
            node: "west",
            seed: 3,
            next_attempt: 2,
            elapsed: elapsed(3, 2),
        };
        let events = plan_replica_retry_timeline_from_checkpoints(
            &[checkpoint],
            33,
            100,
            1000,
            checkpoint.elapsed,
        )
        .unwrap();
        assert!(events.is_empty());
        let empty = ReplicaRetryCheckpoint {
            node: "west",
            seed: 3,
            next_attempt: 0,
            elapsed: Duration::ZERO,
        };
        let entire =
            plan_replica_retry_timeline_from_checkpoints(&[empty], 33, 100, 1000, Duration::MAX)
                .unwrap();
        assert_eq!(
            entire,
            plan_replica_retry_timeline(&[("west", 3)], 33, 100, 1000, Duration::MAX).unwrap()
        );
    }
