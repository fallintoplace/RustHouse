use super::*;

fn object(key: &str, bytes: u64) -> ObjectMetadata {
    ObjectMetadata {
        key: key.into(),
        bytes,
        crc32c: bytes as u32 ^ 0x1234_5678,
    }
}

#[test]
fn both_directions_match_independent_copy_plans() {
    for length in [0usize, 1, 8, 9, 257, 1025] {
        let source = (0..length)
            .filter(|i| i % 3 != 0)
            .map(|i| object(&format!("src/{i:05}"), i as u64))
            .collect::<Vec<_>>();
        let destination = (0..length)
            .filter(|i| i % 4 != 0)
            .map(|i| object(&format!("src/{i:05}"), i as u64))
            .collect::<Vec<_>>();
        let expected_copies = plan_replica_copies(&source, &destination).unwrap();
        let expected_orphans = plan_replica_copies(&destination, &source).unwrap();
        for reverse_source in [false, true] {
            for reverse_destination in [false, true] {
                let mut a = source.clone();
                let mut b = destination.clone();
                if reverse_source {
                    a.reverse();
                }
                if reverse_destination {
                    b.reverse();
                }
                let delta = reconcile_replica_inventories(&a, &b).unwrap();
                assert_eq!(delta.copies, expected_copies);
                assert_eq!(delta.destination_only, expected_orphans);
            }
        }
    }
}

#[test]
fn conflicts_unsafe_keys_and_duplicates_reject_entire_result() {
    let valid = object("part/data.bin", 10);
    let different = object("part/data.bin", 11);
    assert!(reconcile_replica_inventories(std::slice::from_ref(&valid), &[different]).is_err());
    assert!(reconcile_replica_inventories(&[valid.clone(), valid], &[]).is_err());
    assert!(reconcile_replica_inventories(&[], &[object("../outside", 1)]).is_err());
    assert!(reconcile_replica_inventories(&[], &[object("part/CON.txt", 1)]).is_err());
}
