use super::*;

fn object(key: &str) -> ObjectMetadata {
    ObjectMetadata {
        key: key.to_owned(),
        bytes: 13,
        crc32c: 0x1234,
    }
}

#[test]
fn rejects_case_aliases_within_one_replica_and_across_replicas() {
    let left = [object("region/shard/g1/part/FOO.bin")];
    let right = [object("region/shard/g1/part/foo.bin")];
    assert!(plan_replica_copies(&left, &right).is_err());
    assert!(plan_replica_copy_batch_after(&left, &right, None, 100, 1).is_err());
    assert!(ReplicaCopyPlanner::new(&left, &right).is_err());
    assert!(compare_replica_inventories(&left, &right).is_err());

    let two = [
        object("region/shard/g1/part/FOO.bin"),
        object("region/shard/g1/part/foo.bin"),
    ];
    assert!(plan_replica_copies(&two, &[]).is_err());
    assert!(ReplicaCopyPlanner::new(&two, &[]).is_err());
}

#[test]
fn rejects_aliases_in_large_sorted_and_unsorted_inventories() {
    let mut source = (0..128)
        .map(|id| object(&format!("cluster/shard/g1/part/key{id:04}.bin")))
        .collect::<Vec<_>>();
    let mut destination = source.clone();
    source.push(object("cluster/shard/g1/part/Mixed.bin"));
    destination.push(object("cluster/shard/g1/part/mixed.bin"));
    assert!(plan_replica_copies(&source, &destination).is_err());
    assert!(ReplicaCopyPlanner::new(&source, &destination).is_err());
    source.reverse();
    destination.reverse();
    assert!(plan_replica_copy_batch_after(&source, &destination, None, 1000, 2).is_err());
    assert!(compare_replica_inventories(&source, &destination).is_err());
}

#[test]
fn accepts_identical_mixed_case_names_on_both_sides() {
    let same = [object("Prod/Shard/g1/Part/Data.BIN")];
    assert!(plan_replica_copies(&same, &same).unwrap().is_empty());
    assert!(ReplicaCopyPlanner::new(&same, &same).unwrap().is_empty());
    assert_eq!(
        compare_replica_inventories(&same, &same).unwrap(),
        ReplicaInventoryDifference::default()
    );
}
