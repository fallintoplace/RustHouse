use super::*;

#[test]
fn large_immutable_objects_do_not_hide_later_affordable_copies() {
    let source = (0..8_192)
        .rev()
        .map(|index| ObjectMetadata {
            key: format!("part/file_{index:05}"),
            bytes: if index % 101 == 0 {
                0
            } else if index % 17 == 0 {
                1
            } else {
                1_000_000
            },
            crc32c: index,
        })
        .collect::<Vec<_>>();
    let full = plan_replica_copies(&source, &[]).unwrap();
    for budget in [0, 1, 2, 8, 64] {
        for limit in [2, 3, 5, 16, 64, 257] {
            let mut remaining = budget;
            let expected = full
                .iter()
                .filter(|object| {
                    if object.bytes > remaining {
                        return false;
                    }
                    remaining -= object.bytes;
                    true
                })
                .take(limit)
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(
                plan_replica_copy_batch(&source, &[], budget, limit).unwrap(),
                expected,
                "budget={budget} limit={limit}",
            );
        }
    }
}
