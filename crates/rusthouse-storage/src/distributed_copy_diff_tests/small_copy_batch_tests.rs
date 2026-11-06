use super::*;

#[test]
fn tiny_batches_match_full_sorted_greedy_selection() {
    let source = (0..8u64)
        .rev()
        .map(|index| ObjectMetadata {
            key: format!("part/file_{index:02}"),
            bytes: index % 4,
            crc32c: index as u32,
        })
        .collect::<Vec<_>>();
    for existing in 0..=8 {
        let destination = source.iter().take(existing).cloned().collect::<Vec<_>>();
        let missing = plan_replica_copies(&source, &destination).unwrap();
        let references = missing.iter().collect::<Vec<_>>();
        for budget in [0, 1, 2, 3, 10, u64::MAX] {
            for limit in 0..=9 {
                assert_eq!(
                    plan_replica_copy_batch(&source, &destination, budget, limit).unwrap(),
                    select_copy_batch(&references, budget, limit),
                );
            }
        }
    }
}

#[test]
fn zero_limit_still_detects_conflicts_and_duplicate_keys() {
    let a = ObjectMetadata {
        key: "part/a".into(),
        bytes: 1,
        crc32c: 3,
    };
    let different = ObjectMetadata {
        bytes: 2,
        ..a.clone()
    };
    assert!(plan_replica_copy_batch(std::slice::from_ref(&a), &[different], 0, 0).is_err());
    assert!(plan_replica_copy_batch(&[a.clone(), a], &[], 0, 0).is_err());
}
