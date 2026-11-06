use super::*;

fn entry(index: usize) -> ObjectMetadata {
    ObjectMetadata {
        key: format!("cluster/shard/g1/part/file{index:05}"),
        bytes: index as u64,
        crc32c: index as u32 ^ 0x1234,
    }
}

#[test]
fn sorted_merge_and_unordered_hash_plans_choose_the_same_objects() {
    let source = (0..1024).map(entry).collect::<Vec<_>>();
    for step in [1usize, 2, 3, 7, 17, 127, 1024] {
        let destination = source
            .iter()
            .enumerate()
            .filter(|(i, _)| i % step != 0)
            .map(|(_, object)| object.clone())
            .collect::<Vec<_>>();
        let expected = source
            .iter()
            .enumerate()
            .filter(|(i, _)| i % step == 0)
            .map(|(_, object)| object.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            plan_replica_copies(&source, &destination).unwrap(),
            expected
        );
        let mut reversed_source = source.clone();
        reversed_source.reverse();
        let mut reversed_destination = destination.clone();
        reversed_destination.reverse();
        assert_eq!(
            plan_replica_copies(&reversed_source, &reversed_destination).unwrap(),
            expected
        );
    }
}

#[test]
fn sorted_planner_rejects_duplicate_invalid_and_conflicting_objects() {
    let source = (0..16).map(entry).collect::<Vec<_>>();
    let mut conflict = source.clone();
    conflict[7].crc32c ^= 1;
    assert!(plan_replica_copies(&source, &conflict).is_err());
    let mut duplicate = source.clone();
    let repeated = duplicate[3].clone();
    duplicate.insert(3, repeated);
    assert!(plan_replica_copies(&duplicate, &[]).is_err());
    let mut unsafe_key = source.clone();
    unsafe_key[9].key = "cluster/../bad".into();
    assert!(plan_replica_copies(&unsafe_key, &[]).is_err());
}
