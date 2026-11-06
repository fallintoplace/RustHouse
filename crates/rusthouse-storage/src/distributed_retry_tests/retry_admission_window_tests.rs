    use super::*;

    fn event(node: &str, attempt: u32, millis: u64) -> ReplicaRetryEvent {
        ReplicaRetryEvent {
            node: node.into(),
            attempt,
            due_after: Duration::from_millis(millis),
        }
    }

    #[test]
    fn respects_global_and_per_node_windows_at_exact_boundaries() {
        let events = [
            event("b", 0, 0),
            event("a", 0, 0),
            event("a", 1, 1),
            event("b", 1, 50),
            event("a", 2, 100),
            event("b", 2, 100),
        ];
        let selected =
            admit_retry_events_with_quotas(&events, Duration::from_millis(100), 2, 1).unwrap();
        assert_eq!(
            selected,
            [
                event("a", 0, 0),
                event("b", 0, 0),
                event("a", 2, 100),
                event("b", 2, 100)
            ]
        );
    }

    #[test]
    fn scheduling_is_invariant_under_event_input_order() {
        let mut events = (0..33)
            .flat_map(|attempt| {
                (0..64)
                    .map(move |node| event(&format!("node_{node:02}"), attempt, attempt as u64 * 5))
            })
            .collect::<Vec<_>>();
        let first =
            admit_retry_events_with_quotas(&events, Duration::from_millis(20), 12, 2).unwrap();
        events.reverse();
        let second =
            admit_retry_events_with_quotas(&events, Duration::from_millis(20), 12, 2).unwrap();
        assert_eq!(first, second);
        let mut global = std::collections::VecDeque::new();
        let mut per_node = std::collections::HashMap::<&str, Vec<u64>>::new();
        for selected in &first {
            let now = selected.due_after.as_millis() as u64;
            while global.front().is_some_and(|&older| now - older >= 20) {
                global.pop_front();
            }
            assert!(global.len() < 12);
            global.push_back(now);
            let times = per_node.entry(selected.node.as_str()).or_default();
            times.retain(|&older| now - older < 20);
            assert!(times.len() < 2);
            times.push(now);
        }
    }

    #[test]
    fn rejects_bad_batches_before_any_admission() {
        let ok = event("a", 0, 5);
        assert!(
            admit_retry_events_with_quotas(&[ok.clone(), ok], Duration::from_secs(1), 2, 1)
                .is_err()
        );
        assert!(
            admit_retry_events_with_quotas(&[event("../a", 0, 0)], Duration::from_secs(1), 2, 1)
                .is_err()
        );
        assert!(admit_retry_events_with_quotas(&[], Duration::ZERO, 1, 1).is_err());
        assert!(admit_retry_events_with_quotas(&[], Duration::from_secs(1), 0, 1).is_err());
        assert!(
            admit_retry_events_with_quotas(&[event("a", 33, 0)], Duration::from_secs(1), 1, 1)
                .is_err()
        );
    }
