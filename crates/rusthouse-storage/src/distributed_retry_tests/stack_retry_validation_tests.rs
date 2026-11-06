use super::*;

#[test]
fn maximum_fanout_preserves_sorted_retry_timeline_and_duplicate_rejection() {
    let names = (0..64)
        .map(|index| format!("replica_{index:02}"))
        .collect::<Vec<_>>();
    let members = names
        .iter()
        .enumerate()
        .map(|(index, node)| (node.as_str(), index as u64 + 7))
        .collect::<Vec<_>>();
    let schedule =
        plan_replica_retry_timeline(&members, 3, 10, 100, Duration::from_secs(1)).unwrap();
    assert_eq!(schedule.len(), 64 * 3);
    assert!(schedule.windows(2).all(|pair| {
        pair[0].due_after < pair[1].due_after
            || (pair[0].due_after == pair[1].due_after && pair[0].node <= pair[1].node)
    }));
    let mut duplicate = members.clone();
    duplicate[63].0 = duplicate[0].0;
    assert!(plan_replica_retry_timeline(&duplicate, 0, 10, 100, Duration::ZERO).is_err());
}

#[test]
fn too_short_deadlines_skip_heap_work_but_still_validate_inputs() {
    let members = [("one", 1), ("two", 2)];
    assert!(
        plan_replica_retry_timeline(&members, 33, 100, 1000, Duration::from_millis(49))
            .unwrap()
            .is_empty()
    );
    assert!(plan_replica_retry_timeline(&members, 0, 0, 1000, Duration::ZERO).is_err());
    assert!(
        plan_replica_retry_timeline(&[("bad/name", 1)], 0, 100, 1000, Duration::ZERO).is_err()
    );
}
