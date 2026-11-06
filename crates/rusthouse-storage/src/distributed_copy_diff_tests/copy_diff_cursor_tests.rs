use super::*;

#[test]
fn paged_batches_follow_the_full_sorted_plan_for_tiny_and_large_inventories() {
    let source = (0..4096usize)
        .rev()
        .map(|index| ObjectMetadata {
            key: format!("part/object_{index:05}"),
            bytes: if index % 19 == 0 {
                0
            } else {
                (index % 127) as u64
            },
            crc32c: index as u32,
        })
        .collect::<Vec<_>>();
    let destination = source.iter().step_by(7).cloned().collect::<Vec<_>>();
    let full = plan_replica_copies(&source, &destination).unwrap();
    for cursor in [
        None,
        Some("part/object_00000"),
        Some("part/object_01234"),
        Some("part/object_04095"),
        Some("part/object_99999"),
    ] {
        for budget in [0, 1, 100, 1000, u64::MAX] {
            for limit in [0, 1, 2, 7, 100] {
                let mut remaining = budget;
                let expected = full
                    .iter()
                    .filter(|object| cursor.is_none_or(|key| object.key.as_str() > key))
                    .filter(|object| {
                        if object.bytes <= remaining {
                            remaining -= object.bytes;
                            true
                        } else {
                            false
                        }
                    })
                    .take(limit)
                    .cloned()
                    .collect::<Vec<_>>();
                assert_eq!(
                    plan_replica_copy_batch_after(&source, &destination, cursor, budget, limit)
                        .unwrap(),
                    expected
                );
            }
        }
    }
}

#[test]
fn cursor_is_validated_even_for_empty_pages() {
    assert!(plan_replica_copy_batch_after(&[], &[], Some("../bad"), 0, 0).is_err());
    assert_eq!(
        plan_replica_copy_batch_after(&[], &[], None, 0, 0).unwrap(),
        Vec::<ObjectMetadata>::new()
    );
}
