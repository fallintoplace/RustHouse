//! Copy-only reconciliation plan for trusted immutable object inventories.
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub key: String,
    pub bytes: u64,
    pub crc32c: u32,
}

impl ObjectMetadata {
    fn same_contents(&self, other: &Self) -> bool {
        self.bytes == other.bytes && self.crc32c == other.crc32c
    }
}

fn valid_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    if bytes.is_empty() || bytes.len() > 512 {
        return false;
    }
    let mut segment_start = true;
    let mut previous_dot = false;
    for &byte in bytes {
        if byte == b'/' {
            if segment_start {
                return false;
            }
            segment_start = true;
            previous_dot = false;
            continue;
        }
        if (segment_start && !byte.is_ascii_alphanumeric())
            || (!byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-' | b'.'))
            || (byte == b'.' && previous_dot)
        {
            return false;
        }
        segment_start = false;
        previous_dot = byte == b'.';
    }
    !segment_start
}

fn index(objects: &[ObjectMetadata]) -> Result<HashMap<&str, &ObjectMetadata>, &'static str> {
    if objects.len() > 65_536 {
        return Err("replica inventory too large");
    }
    let mut indexed = HashMap::with_capacity(objects.len());
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
    let mut changed: Vec<ObjectMetadata> = source
        .into_values()
        .filter(|meta| {
            destination
                .get(meta.key.as_str())
                .is_none_or(|existing| !meta.same_contents(existing))
        })
        .cloned()
        .collect();
    changed.sort_unstable_by(|left, right| left.key.cmp(&right.key));
    Ok(changed)
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
}
