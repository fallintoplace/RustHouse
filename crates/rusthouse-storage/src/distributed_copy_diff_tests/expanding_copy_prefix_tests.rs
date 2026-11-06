use super::*;

#[test]
fn tiny_batches_match_full_sorted_greedy_plan_at_inventory_scale() {
    let source = (0..8_192usize)
        .rev()
        .map(|index| ObjectMetadata {
            key: format!("part/object_{index:05}"),
            bytes: if index % 17 == 0 {
                0
            } else {
                (index % 257) as u64
            },
            crc32c: index as u32,
        })
        .collect::<Vec<_>>();
    let destination = source.iter().step_by(7).cloned().collect::<Vec<_>>();
    let full = plan_replica_copies(&source, &destination).unwrap();
    for budget in [0, 1, 23, 100, 1024, u64::MAX] {
        for limit in [2, 3, 8, 31, 127] {
            let mut remaining = budget;
            let expected = full
                .iter()
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
                plan_replica_copy_batch(&source, &destination, budget, limit).unwrap(),
                expected,
                "budget={budget}, limit={limit}",
            );
        }
    }
}
