use super::*;

#[test]
fn rejects_reserved_device_names_in_any_inventory_path_segment() {
    for key in [
        "NUL",
        "p/CON.txt",
        "a/com1.bin",
        "a/b/LPT9.log",
        "prn.csv/part",
        "AUX",
    ] {
        let value = ObjectMetadata {
            key: key.to_owned(),
            bytes: 1,
            crc32c: 0,
        };
        assert!(
            plan_replica_copies(std::slice::from_ref(&value), &[]).is_err(),
            "{key}"
        );
        assert!(
            plan_replica_copy_batch(&[value], &[], 1, 1).is_err(),
            "{key}"
        );
    }
    for key in ["a/CONTEXT.txt", "x/com10.bin", "z/LPT0.log"] {
        let value = ObjectMetadata {
            key: key.to_owned(),
            bytes: 1,
            crc32c: 0,
        };
        assert_eq!(
            plan_replica_copies(std::slice::from_ref(&value), &[]).unwrap(),
            vec![value]
        );
    }
}
