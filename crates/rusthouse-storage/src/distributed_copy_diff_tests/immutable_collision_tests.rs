use super::*;

#[test]
fn batch_refuses_same_key_with_different_contents() {
    let source = [ObjectMetadata {
        key: "p/g1/id.bin".into(),
        bytes: 10,
        crc32c: 1,
    }];
    let conflicting = [ObjectMetadata {
        key: "p/g1/id.bin".into(),
        bytes: 10,
        crc32c: 2,
    }];
    assert!(plan_replica_copy_batch(&source, &conflicting, 100, 10).is_err());
    assert!(
        plan_replica_copy_batch(&source, &source, 100, 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        plan_replica_copy_batch(&source, &[], 100, 10).unwrap(),
        vec![source[0].clone()]
    );
}
