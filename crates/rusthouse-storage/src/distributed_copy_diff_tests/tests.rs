use super::*;

#[test]
fn plans_missing_objects_but_rejects_immutable_conflicts() {
    let objects = vec![
        ObjectMetadata {
            key: "s/g1/a".into(),
            bytes: 12,
            crc32c: 1,
        },
        ObjectMetadata {
            key: "s/g1/b".into(),
            bytes: 6,
            crc32c: 2,
        },
    ];
    let existing = vec![
        objects[0].clone(),
        ObjectMetadata {
            key: "s/g0/old".into(),
            bytes: 1,
            crc32c: 0,
        },
    ];
    assert_eq!(
        plan_replica_copies(&objects, &existing).unwrap(),
        vec![objects[1].clone()]
    );
    let mut conflicting = existing;
    conflicting.push(ObjectMetadata {
        key: "s/g1/b".into(),
        bytes: 6,
        crc32c: 3,
    });
    assert!(plan_replica_copies(&objects, &conflicting).is_err());
    assert!(plan_replica_copy_batch(&objects, &conflicting, 100, 2).is_err());
    assert!(plan_replica_copies(&objects, &objects).unwrap().is_empty());
}

#[test]
fn rejects_hidden_path_segments() {
    for key in [".hidden/file", "part/.private", "part/é"] {
        let object = ObjectMetadata {
            key: key.into(),
            bytes: 1,
            crc32c: 0,
        };
        assert!(plan_replica_copies(&[object], &[]).is_err(), "{key}");
    }
    let valid = ObjectMetadata {
        key: "part/checksums.txt".into(),
        bytes: 1,
        crc32c: 0,
    };
    assert_eq!(
        plan_replica_copies(std::slice::from_ref(&valid), &[]).unwrap(),
        vec![valid]
    );
}

#[test]
fn rejects_duplicate_and_traversing_keys() {
    let object = ObjectMetadata {
        key: "a/b".into(),
        bytes: 1,
        crc32c: 0,
    };
    assert!(plan_replica_copies(&[object.clone(), object], &[]).is_err());
    assert!(
        plan_replica_copies(
            &[ObjectMetadata {
                key: "a/../b".into(),
                bytes: 1,
                crc32c: 0
            }],
            &[],
        )
        .is_err()
    );
}
#[test]
fn copy_plan_is_sorted_even_when_source_order_is_reversed() {
    let source = vec![
        ObjectMetadata {
            key: "z/file".into(),
            bytes: 2,
            crc32c: 20,
        },
        ObjectMetadata {
            key: "a/file".into(),
            bytes: 1,
            crc32c: 10,
        },
        ObjectMetadata {
            key: "m/file".into(),
            bytes: 3,
            crc32c: 30,
        },
    ];
    let destination = vec![source[0].clone()];
    let plan = plan_replica_copies(&source, &destination).unwrap();
    assert_eq!(
        plan.iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>(),
        vec!["a/file", "m/file"]
    );
}
