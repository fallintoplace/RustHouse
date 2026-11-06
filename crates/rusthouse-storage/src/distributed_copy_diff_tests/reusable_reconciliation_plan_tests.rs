use super::*;

#[test]
fn repeated_pages_match_one_shot_copy_planner_for_skewed_inventories() {
    let source = (0..8_192usize)
        .rev()
        .map(|index| ObjectMetadata {
            key: format!("part/object_{index:05}"),
            bytes: if index % 9 == 0 {
                0
            } else {
                (index % 101) as u64
            },
            crc32c: index as u32,
        })
        .collect::<Vec<_>>();
    let destination = source.iter().step_by(5).cloned().collect::<Vec<_>>();
    let planner = ReplicaCopyPlanner::new(&source, &destination).unwrap();
    assert_eq!(planner.len(), source.len() - destination.len());
    assert!(!planner.is_empty());

    for cursor in [
        None,
        Some("part/object_00000"),
        Some("part/object_04096"),
        Some("part/object_08191"),
    ] {
        for budget in [0u64, 1, 49, 255, 4096, u64::MAX] {
            for limit in [0usize, 1, 2, 17, 127] {
                assert_eq!(
                    planner.batch_after(cursor, budget, limit).unwrap(),
                    plan_replica_copy_batch_after(&source, &destination, cursor, budget, limit)
                        .unwrap(),
                    "cursor={cursor:?} budget={budget} limit={limit}"
                );
            }
        }
    }
}

#[test]
fn invalid_inventories_and_cursor_rejected_without_mutations() {
    let object = ObjectMetadata {
        key: "safe/one".into(),
        bytes: 3,
        crc32c: 10,
    };
    let source = [object.clone()];
    assert!(ReplicaCopyPlanner::new(&[object.clone(), object.clone()], &[]).is_err());
    assert!(
        ReplicaCopyPlanner::new(
            &source,
            &[ObjectMetadata {
                crc32c: 11,
                ..object.clone()
            }]
        )
        .is_err()
    );
    let planner = ReplicaCopyPlanner::new(&source, &[]).unwrap();
    assert_eq!(planner.batch_after(None, 3, 1).unwrap(), source);
    assert!(planner.batch_after(Some("../invalid"), 3, 1).is_err());
    assert!(planner.batch_after(None, 3, 65_537).is_err());
    assert!(ReplicaCopyPlanner::new(&[], &[]).unwrap().is_empty());
}
