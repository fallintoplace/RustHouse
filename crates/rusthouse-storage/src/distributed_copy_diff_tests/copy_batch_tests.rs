use super::*;

#[test]
fn bounded_batch_matches_full_plan_for_many_objects() {
    let source = (0..500)
        .rev()
        .map(|index| ObjectMetadata {
            key: format!("part/{index:04}.bin"),
            bytes: if index % 7 == 0 { 500 } else { 1 },
            crc32c: index,
        })
        .collect::<Vec<_>>();
    let expected = plan_replica_copies(&source, &[]).unwrap();
    let mut remaining = 12u64;
    let selected = expected
        .into_iter()
        .filter(|object| {
            if object.bytes > remaining {
                return false;
            }
            remaining -= object.bytes;
            true
        })
        .take(12)
        .collect::<Vec<_>>();
    assert_eq!(
        plan_replica_copy_batch(&source, &[], 12, 12).unwrap(),
        selected
    );
}

#[test]
fn one_copy_batch_matches_the_sorted_plan_without_a_full_sort() {
    let source = (0..512)
        .rev()
        .map(|index| ObjectMetadata {
            key: format!("part/{index:04}.bin"),
            bytes: 1 + (index % 9) as u64,
            crc32c: index,
        })
        .collect::<Vec<_>>();
    let destination = source
        .iter()
        .filter(|object| object.key.ends_with('0'))
        .cloned()
        .collect::<Vec<_>>();
    let all = plan_replica_copies(&source, &destination).unwrap();
    for budget in [0u64, 1, 3, 5, 12] {
        let expected = all.iter().find(|object| object.bytes <= budget).cloned();
        assert_eq!(
            plan_replica_copy_batch(&source, &destination, budget, 1)
                .unwrap()
                .into_iter()
                .next(),
            expected,
        );
    }
    assert!(
        plan_replica_copy_batch(&source, &destination, u64::MAX, 0)
            .unwrap()
            .is_empty()
    );
    let invalid = [ObjectMetadata {
        key: "../bad".into(),
        bytes: 0,
        crc32c: 0,
    }];
    assert!(plan_replica_copy_batch(&invalid, &[], 0, 0).is_err());
}

#[test]
fn selects_bounded_copies_without_deleting_destination_only_objects() {
    let objects = [
        ObjectMetadata {
            key: "a/large".into(),
            bytes: 9,
            crc32c: 1,
        },
        ObjectMetadata {
            key: "b/small".into(),
            bytes: 2,
            crc32c: 2,
        },
        ObjectMetadata {
            key: "c/small".into(),
            bytes: 4,
            crc32c: 3,
        },
    ];
    let batch = plan_replica_copy_batch(&objects, &[], 6, 3).unwrap();
    assert_eq!(
        batch
            .iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>(),
        vec!["b/small", "c/small"]
    );
    assert_eq!(
        plan_replica_copy_batch(&objects, &[], 6, 1).unwrap().len(),
        1
    );
    assert!(plan_replica_copy_batch(&objects, &[], 6, 65_537).is_err());
}
