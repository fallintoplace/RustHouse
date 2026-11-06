use super::*;

#[test]
fn bounded_prefix_matches_full_copy_plan_at_inventory_scale() {
    let source = (0..2048usize)
        .rev()
        .map(|index| ObjectMetadata {
            key: format!("part/{index:05}.bin"),
            bytes: (index % 17) as u64,
            crc32c: index as u32,
        })
        .collect::<Vec<_>>();
    let destination = source
        .iter()
        .filter(|meta| meta.crc32c % 5 == 0)
        .cloned()
        .collect::<Vec<_>>();
    let all = plan_replica_copies(&source, &destination).unwrap();
    for count in [2, 3, 8, 32, 256, 2048, 4096] {
        let selected = plan_replica_copy_batch(&source, &destination, u64::MAX, count).unwrap();
        assert_eq!(
            selected,
            all.iter().take(count).cloned().collect::<Vec<_>>()
        );
    }
}

#[test]
fn tight_budgets_keep_greedy_skips_and_zero_byte_objects() {
    let source = [
        ObjectMetadata {
            key: "a/x".into(),
            bytes: 9,
            crc32c: 0,
        },
        ObjectMetadata {
            key: "b/x".into(),
            bytes: 0,
            crc32c: 0,
        },
        ObjectMetadata {
            key: "c/x".into(),
            bytes: 2,
            crc32c: 0,
        },
        ObjectMetadata {
            key: "d/x".into(),
            bytes: 1,
            crc32c: 0,
        },
        ObjectMetadata {
            key: "e/x".into(),
            bytes: 1,
            crc32c: 0,
        },
    ];
    assert_eq!(
        plan_replica_copy_batch(&source, &[], 3, 2)
            .unwrap()
            .iter()
            .map(|meta| meta.key.as_str())
            .collect::<Vec<_>>(),
        vec!["b/x", "c/x"]
    );
    assert_eq!(
        plan_replica_copy_batch(&source, &[], 0, 3)
            .unwrap()
            .iter()
            .map(|meta| meta.key.as_str())
            .collect::<Vec<_>>(),
        vec!["b/x"]
    );
    // An enormous missing object must not overflow the prefix sum.
    let huge = [
        ObjectMetadata {
            key: "a/x".into(),
            bytes: u64::MAX,
            crc32c: 1,
        },
        ObjectMetadata {
            key: "b/x".into(),
            bytes: 5,
            crc32c: 2,
        },
        ObjectMetadata {
            key: "c/x".into(),
            bytes: 0,
            crc32c: 3,
        },
    ];
    assert_eq!(
        plan_replica_copy_batch(&huge, &[], 5, 2)
            .unwrap()
            .iter()
            .map(|meta| meta.key.as_str())
            .collect::<Vec<_>>(),
        vec!["b/x", "c/x"]
    );
}
