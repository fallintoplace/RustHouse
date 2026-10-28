//! Copy-only reconciliation plan for trusted immutable object inventories.
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub key: String,
    pub bytes: u64,
    pub crc32c: u32,
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 512
        && key.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && !segment.contains("..")
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
}

fn index<'a>(
    objects: &'a [ObjectMetadata],
) -> Result<BTreeMap<&'a str, &'a ObjectMetadata>, &'static str> {
    if objects.len() > 65_536 {
        return Err("replica inventory too large");
    }
    let mut indexed = BTreeMap::new();
    for object in objects {
        if !valid_key(&object.key) || indexed.insert(object.key.as_str(), object).is_some() {
            return Err("unsafe or duplicate replica object key");
        }
    }
    Ok(indexed)
}

/// Return missing or differing source objects in lexicographic key order.
/// Deletions require a separate retention decision and are never planned here.
pub fn plan_replica_copies(
    source: &[ObjectMetadata],
    destination: &[ObjectMetadata],
) -> Result<Vec<ObjectMetadata>, &'static str> {
    let source = index(source)?;
    let destination = index(destination)?;
    Ok(source
        .into_iter()
        .filter(|(key, meta)| {
            destination.get(key).is_none_or(|existing| {
                existing.bytes != meta.bytes || existing.crc32c != meta.crc32c
            })
        })
        .map(|(_, meta)| meta.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_only_missing_and_mismatched_objects() {
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
                key: "s/g1/b".into(),
                bytes: 6,
                crc32c: 3,
            },
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
        assert!(plan_replica_copies(&objects, &objects).unwrap().is_empty());
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
}
