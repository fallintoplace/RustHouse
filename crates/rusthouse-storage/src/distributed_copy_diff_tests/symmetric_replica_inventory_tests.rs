use super::*;

fn item(key: &str, bytes: u64, crc32c: u32) -> ObjectMetadata {
    ObjectMetadata {
        key: key.into(),
        bytes,
        crc32c,
    }
}

#[test]
fn diagnoses_conflicts_and_missing_objects_without_authorizing_overwrite() {
    let left = [
        item("cluster/shard/g1/part/a.bin", 10, 1),
        item("cluster/shard/g1/part/b.bin", 20, 2),
        item("cluster/shard/g1/part/c.bin", 30, 3),
    ];
    let right = [
        item("cluster/shard/g1/part/d.bin", 40, 4),
        item("cluster/shard/g1/part/c.bin", 30, 3),
        item("cluster/shard/g1/part/b.bin", 20, 9),
    ];
    let difference = compare_replica_inventories(&left, &right).unwrap();
    assert_eq!(difference.only_left, [left[0].clone()]);
    assert_eq!(difference.only_right, [right[0].clone()]);
    assert_eq!(
        difference.conflicting,
        [ReplicaObjectConflict {
            key: left[1].key.clone(),
            left_bytes: 20,
            left_crc32c: 2,
            right_bytes: 20,
            right_crc32c: 9,
        }]
    );
    assert!(plan_replica_copies(&left, &right).is_err());

    let reversed = compare_replica_inventories(&right, &left).unwrap();
    assert_eq!(difference.only_left, reversed.only_right);
    assert_eq!(difference.only_right, reversed.only_left);
    assert_eq!(reversed.conflicting[0].left_crc32c, 9);
    assert_eq!(reversed.conflicting[0].right_crc32c, 2);
}

#[test]
fn sorted_diagnostics_match_across_large_reversed_inventories() {
    let left = (0..128)
        .rev()
        .map(|n| {
            item(
                &format!("cluster/shard/g1/part/left_{n:03}.bin"),
                n,
                n as u32,
            )
        })
        .collect::<Vec<_>>();
    let right = (0..128)
        .map(|n| {
            item(
                &format!("cluster/shard/g1/part/right_{n:03}.bin"),
                n,
                n as u32,
            )
        })
        .collect::<Vec<_>>();
    let result = compare_replica_inventories(&left, &right).unwrap();
    assert_eq!(result.only_left.len(), 128);
    assert_eq!(result.only_right.len(), 128);
    assert_eq!(
        result.only_left[0].key,
        "cluster/shard/g1/part/left_000.bin"
    );
    assert_eq!(
        result.only_left[127].key,
        "cluster/shard/g1/part/left_127.bin"
    );
    assert!(result.conflicting.is_empty());
}

#[test]
fn sorted_two_cursor_diff_matches_hash_reference_for_mixed_inventories() {
    let left = (0..1_024u64)
        .map(|index| {
            item(
                &format!("cluster/shard/g1/part/key_{index:04}.bin"),
                index,
                index as u32,
            )
        })
        .collect::<Vec<_>>();
    let right = (0..1_024u64)
        .filter(|index| index % 7 != 0)
        .map(|index| {
            item(
                &format!("cluster/shard/g1/part/key_{index:04}.bin"),
                index,
                (index as u32) ^ u32::from(index % 5 == 0),
            )
        })
        .chain([item("cluster/shard/g1/part/new.bin", 9, 99)])
        .collect::<Vec<_>>();
    let sorted = compare_replica_inventories(&left, &right).unwrap();
    let reversed_left = left.iter().cloned().rev().collect::<Vec<_>>();
    let reversed_right = right.iter().cloned().rev().collect::<Vec<_>>();
    let hashed = compare_replica_inventories(&reversed_left, &reversed_right).unwrap();
    assert_eq!(sorted, hashed);
    assert!(!sorted.only_left.is_empty());
    assert!(!sorted.only_right.is_empty());
    assert!(!sorted.conflicting.is_empty());
}

#[test]
fn refuses_ambiguous_key_or_duplicate_on_either_side() {
    let good = item("cluster/shard/g1/part/data.bin", 1, 2);
    assert!(compare_replica_inventories(&[good.clone(), good.clone()], &[],).is_err());
    assert!(compare_replica_inventories(&[], &[good.clone(), good.clone()],).is_err());
    assert!(compare_replica_inventories(&[item("../escaped", 1, 0)], &[],).is_err());
}
