use super::*;

#[test]
fn rejects_oversized_path_components_without_rejecting_long_valid_keys() {
    for key in [
        "x".repeat(129),
        format!("prefix/{}", "a".repeat(129)),
        format!("{}/suffix", "z".repeat(129)),
    ] {
        let meta = ObjectMetadata {
            key,
            bytes: 1,
            crc32c: 0,
        };
        assert!(plan_replica_copies(std::slice::from_ref(&meta), &[]).is_err());
        assert!(plan_replica_copy_batch(&[meta], &[], 1, 1).is_err());
    }
    let key = format!("{}/{}", "a".repeat(128), "b".repeat(128));
    let meta = ObjectMetadata {
        key,
        bytes: 1,
        crc32c: 0,
    };
    assert_eq!(
        plan_replica_copies(std::slice::from_ref(&meta), &[]).unwrap(),
        vec![meta]
    );
}
