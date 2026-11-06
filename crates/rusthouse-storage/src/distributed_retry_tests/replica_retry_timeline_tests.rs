    use super::*;

    #[test]
    fn each_node_follows_exact_seeded_backoff_and_global_deadline() {
        let nodes = [("west", 3), ("east", 99), ("south", u64::MAX)];
        let deadline = Duration::from_secs(9);
        let timeline = plan_replica_retry_timeline(&nodes, 33, 100, 2000, deadline).unwrap();
        assert!(!timeline.is_empty());
        assert!(timeline.iter().all(|event| event.due_after <= deadline));
        assert!(timeline.windows(2).all(|pair| {
            (pair[0].due_after, pair[0].node.as_str(), pair[0].attempt)
                <= (pair[1].due_after, pair[1].node.as_str(), pair[1].attempt)
        }));
        for &(node, seed) in &nodes {
            let actual = timeline
                .iter()
                .filter(|event| event.node == node)
                .collect::<Vec<_>>();
            let expected = plan_retry_delays(33, 100, 2000, seed, deadline).unwrap();
            let mut elapsed = Duration::ZERO;
            assert_eq!(actual.len(), expected.len());
            for (index, (event, delay)) in actual.iter().zip(expected).enumerate() {
                elapsed += delay;
                assert_eq!(event.attempt, index as u32);
                assert_eq!(event.due_after, elapsed);
            }
        }
        let mut reversed = nodes;
        reversed.reverse();
        assert_eq!(
            timeline,
            plan_replica_retry_timeline(&reversed, 33, 100, 2000, deadline).unwrap(),
        );
    }

    #[test]
    fn boundary_deadlines_and_invalid_configuration_are_strict() {
        let nodes = [("a", 0)];
        let first = retry_delay(0, 100, 1000, 0).unwrap();
        assert!(
            plan_replica_retry_timeline(&nodes, 4, 100, 1000, Duration::ZERO)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            plan_replica_retry_timeline(&nodes, 4, 100, 1000, first)
                .unwrap()
                .len(),
            1
        );
        assert!(plan_replica_retry_timeline(&nodes, 34, 100, 1000, first).is_err());
        assert!(plan_replica_retry_timeline(&nodes, 0, 0, 1000, first).is_err());
        assert!(plan_replica_retry_timeline(&[("a", 0), ("a", 1)], 0, 100, 1000, first).is_err());
        assert!(plan_replica_retry_timeline(&[("bad/name", 0)], 0, 100, 1000, first).is_err());
    }
