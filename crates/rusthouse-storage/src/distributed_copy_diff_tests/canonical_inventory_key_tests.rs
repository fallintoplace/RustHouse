use super::*;

#[test]
fn rejects_trailing_dots_in_any_path_segment() {
    for path in ["part./file", "part/file.", "part./nested/file"] {
        let object = ObjectMetadata {
            key: path.into(),
            bytes: 1,
            crc32c: 0,
        };
        assert!(plan_replica_copies(&[object], &[]).is_err(), "{path}");
    }
    let valid = ObjectMetadata {
        key: "part/data.bin".into(),
        bytes: 1,
        crc32c: 0,
    };
    assert_eq!(
        plan_replica_copies(std::slice::from_ref(&valid), &[]).unwrap(),
        vec![valid]
    );
}
