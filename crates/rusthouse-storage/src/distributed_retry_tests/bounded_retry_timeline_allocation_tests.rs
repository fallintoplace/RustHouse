    use super::*;

    #[test]
    fn a_zero_deadline_does_not_allocate_thousands_of_event_slots() {
        let owned = (0..64)
            .map(|index| format!("node_{index:02}"))
            .collect::<Vec<_>>();
        let nodes = owned
            .iter()
            .enumerate()
            .map(|(index, name)| (name.as_str(), index as u64))
            .collect::<Vec<_>>();
        let none = plan_replica_retry_timeline(&nodes, 33, 100, 2000, Duration::ZERO).unwrap();
        assert!(none.is_empty());
        assert_eq!(none.capacity(), 0);

        let full = plan_replica_retry_timeline(&nodes, 33, 1, 2, Duration::MAX).unwrap();
        assert_eq!(full.len(), 64 * 33);
        assert!(
            full.windows(2)
                .all(|pair| pair[0].due_after <= pair[1].due_after)
        );
    }

    #[test]
    fn stack_validation_rejects_late_duplicate_and_invalid_node_names() {
        let owned = (0..64)
            .map(|index| format!("node_{index:02}"))
            .collect::<Vec<_>>();
        let mut nodes = owned
            .iter()
            .map(|name| (name.as_str(), 0u64))
            .collect::<Vec<_>>();
        let duplicate = nodes[0].0;
        nodes[63].0 = duplicate;
        assert!(plan_replica_retry_timeline(&nodes, 0, 1, 10, Duration::ZERO).is_err());
        nodes[63].0 = "invalid/node";
        assert!(plan_replica_retry_timeline(&nodes, 0, 1, 10, Duration::ZERO).is_err());
        nodes[63].0 = owned[63].as_str();
        assert!(plan_replica_retry_timeline(&nodes, 0, 1, 10, Duration::ZERO).is_ok());
    }
