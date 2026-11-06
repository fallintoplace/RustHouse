use super::*;

fn item(key: &str, bytes: u64) -> ObjectMetadata {
    ObjectMetadata {
        key: key.into(),
        bytes,
        crc32c: bytes as u32,
    }
}

#[test]
fn small_sources_keep_sorted_missing_keys_without_hash_maps() {
    let source = [item("z/part", 2), item("a/part", 1), item("m/part", 3)];
    let destination = [item("m/part", 3)];
    let changed = plan_replica_copies(&source, &destination).unwrap();
    assert_eq!(changed, vec![item("a/part", 1), item("z/part", 2)]);
    assert!(plan_replica_copies(&source, &source).unwrap().is_empty());
}

#[test]
fn tiny_and_large_paths_reject_conflicts_and_duplicates() {
    let source = [item("a/part", 1), item("b/part", 2)];
    assert!(plan_replica_copies(&source, &[item("a/part", 7)]).is_err());
    assert!(plan_replica_copies(&source, &[item("a/part", 1), item("a/part", 1)]).is_err());
    assert!(plan_replica_copies(&[item("../unsafe", 2)], &[]).is_err());
    let many = (0..1024)
        .map(|index| item(&format!("k{index:04}"), index))
        .collect::<Vec<_>>();
    let result = plan_replica_copies(&many, &many[..512]).unwrap();
    assert_eq!(result.len(), 512);
    assert!(result.windows(2).all(|pair| pair[0].key < pair[1].key));
}
