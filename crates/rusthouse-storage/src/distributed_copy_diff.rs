//! Copy-only reconciliation plan for trusted immutable object inventories.
use std::collections::HashMap;

const MAX_REPLICA_OBJECTS: usize = 65_536;

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
            if segment_start || previous_dot {
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
    !segment_start && !previous_dot
}

fn index(objects: &[ObjectMetadata]) -> Result<HashMap<&str, &ObjectMetadata>, &'static str> {
    if objects.len() > MAX_REPLICA_OBJECTS {
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

/// Select a bounded batch from the deterministic copy-only reconciliation plan.
/// Oversized objects are skipped so smaller copies can still make progress.
pub fn plan_replica_copy_batch(
    source: &[ObjectMetadata],
    destination: &[ObjectMetadata],
    max_bytes: u64,
    max_objects: usize,
) -> Result<Vec<ObjectMetadata>, &'static str> {
    if max_objects > MAX_REPLICA_OBJECTS {
        return Err("replica copy batch exceeds object limit");
    }
    let needed = plan_replica_copies(source, destination)?;
    let mut remaining = max_bytes;
    let mut selected = Vec::with_capacity(needed.len().min(max_objects));
    for object in needed {
        if selected.len() == max_objects {
            break;
        }
        if object.bytes <= remaining {
            remaining -= object.bytes;
            selected.push(object);
        }
    }
    Ok(selected)
}

#[cfg(test)]
mod copy_batch_tests {
    use super::*;

    #[test]
    fn selects_bounded_copies_without_deleting_destination_only_objects() {
        let objects = [
            ObjectMetadata { key: "a/large".into(), bytes: 9, crc32c: 1 },
            ObjectMetadata { key: "b/small".into(), bytes: 2, crc32c: 2 },
            ObjectMetadata { key: "c/small".into(), bytes: 4, crc32c: 3 },
        ];
        let batch = plan_replica_copy_batch(&objects, &[], 6, 3).unwrap();
        assert_eq!(
            batch.iter().map(|item| item.key.as_str()).collect::<Vec<_>>(),
            vec!["b/small", "c/small"]
        );
        assert_eq!(plan_replica_copy_batch(&objects, &[], 6, 1).unwrap().len(), 1);
        assert!(plan_replica_copy_batch(&objects, &[], 6, 65_537).is_err());
    }
}

#[cfg(test)]
mod canonical_inventory_key_tests {
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
        assert_eq!(plan_replica_copies(&[valid.clone()], &[]).unwrap(), vec![valid]);
    }
}
