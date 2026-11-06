use super::*;

fn full_sort_greedy(
    source: &[ObjectMetadata],
    destination: &[ObjectMetadata],
    budget: u64,
) -> Vec<ObjectMetadata> {
    let pending = plan_replica_copies(source, destination).unwrap();
    let mut remaining = budget;
    let mut selected = Vec::new();
    for object in pending {
        if object.bytes <= remaining {
            remaining -= object.bytes;
            selected.push(object);
            if selected.len() == 2 {
                break;
            }
        }
    }
    selected
}

#[test]
fn two_copy_batch_matches_full_sorted_plan_across_budget_boundaries() {
    let source = (0..1024)
        .map(|index| ObjectMetadata {
            key: format!("region/shard_1/g1/part/file_{index:04}.bin"),
            bytes: if index % 17 == 0 {
                0
            } else {
                (index * 97 % 8191) as u64
            },
            crc32c: (index * 7777) as u32,
        })
        .collect::<Vec<_>>();
    let destination = source
        .iter()
        .enumerate()
        .filter(|(index, _)| index % 7 == 0)
        .map(|(_, object)| object.clone())
        .collect::<Vec<_>>();
    for budget in [0, 1, 8, 512, 8192, 100_000, u64::MAX] {
        assert_eq!(
            plan_replica_copy_batch(&source, &destination, budget, 2).unwrap(),
            full_sort_greedy(&source, &destination, budget),
        );
    }
    assert!(
        plan_replica_copy_batch(&source, &destination, 1, 0)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn tiny_batch_still_rejects_existing_immutable_conflicts() {
    let object = ObjectMetadata {
        key: "region/g1/item".into(),
        bytes: 4,
        crc32c: 7,
    };
    let conflict = ObjectMetadata {
        crc32c: 8,
        ..object.clone()
    };
    assert!(
        plan_replica_copy_batch(std::slice::from_ref(&object), &[conflict], 10, 2).is_err()
    );
    assert!(plan_replica_copy_batch(&[object.clone(), object], &[], 10, 2).is_err());
}
