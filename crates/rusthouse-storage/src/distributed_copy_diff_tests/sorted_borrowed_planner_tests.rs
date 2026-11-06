use super::*;

#[test]
fn sorted_inventories_match_unordered_reference_across_pages() {
    let source = (0..2048)
        .map(|index| ObjectMetadata {
            key: format!("part_{index:05}/data.bin"),
            bytes: 1 + (index % 19) as u64,
            crc32c: index,
        })
        .collect::<Vec<_>>();
    let destination = source.iter().step_by(3).cloned().collect::<Vec<_>>();
    let sorted = ReplicaCopyPlanner::new(&source, &destination).unwrap();
    let mut reversed_source = source.clone();
    let mut reversed_destination = destination.clone();
    reversed_source.reverse();
    reversed_destination.reverse();
    let unordered = ReplicaCopyPlanner::new(&reversed_source, &reversed_destination).unwrap();
    assert_eq!(sorted.len(), unordered.len());
    for cursor in [
        None,
        Some("part_00400/data.bin"),
        Some("part_02047/data.bin"),
    ] {
        for limit in [0, 1, 2, 9, 5000] {
            assert_eq!(
                sorted.batch_after(cursor, 500, limit).unwrap(),
                unordered.batch_after(cursor, 500, limit).unwrap()
            );
        }
    }
}

#[test]
fn sorted_planner_rejects_conflicts_and_invalid_keys() {
    let a = ObjectMetadata {
        key: "part_1/data.bin".into(),
        bytes: 1,
        crc32c: 1,
    };
    let b = ObjectMetadata {
        crc32c: 2,
        ..a.clone()
    };
    assert!(ReplicaCopyPlanner::new(std::slice::from_ref(&a), &[b]).is_err());
    assert!(ReplicaCopyPlanner::new(&[a.clone(), a.clone()], &[]).is_err());
    let invalid = ObjectMetadata {
        key: "../oops".into(),
        ..a
    };
    assert!(ReplicaCopyPlanner::new(&[invalid], &[]).is_err());
}
