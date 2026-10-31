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

fn windows_device_segment(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or(segment);
    ["CON", "PRN", "AUX", "NUL"].iter().any(|name| stem.eq_ignore_ascii_case(name))
        || (stem.len() == 4
            && (stem[..3].eq_ignore_ascii_case("COM") || stem[..3].eq_ignore_ascii_case("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

fn valid_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    if bytes.is_empty() || bytes.len() > 512 {
        return false;
    }
    let mut segment_start = true;
    let mut segment_offset = 0usize;
    let mut segment_length = 0usize;
    let mut previous_dot = false;
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'/' {
            if segment_start || previous_dot || windows_device_segment(&key[segment_offset..index]) {
                return false;
            }
            segment_start = true;
            segment_offset = index + 1;
            segment_length = 0;
            previous_dot = false;
            continue;
        }
        segment_length += 1;
        if segment_length > 128 {
            return false;
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
    !segment_start && !previous_dot && !windows_device_segment(&key[segment_offset..])
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

fn reject_immutable_conflicts(
    source: &HashMap<&str, &ObjectMetadata>,
    destination: &HashMap<&str, &ObjectMetadata>,
) -> Result<(), &'static str> {
    if source.iter().any(|(key, object)| {
        destination.get(*key)
            .is_some_and(|existing| !object.same_contents(existing))
    }) {
        return Err("replica copy conflicts with an existing immutable object");
    }
    Ok(())
}

fn needs_copy(meta: &ObjectMetadata, destination: &HashMap<&str, &ObjectMetadata>) -> bool {
    destination
        .get(meta.key.as_str())
        .is_none_or(|existing| !meta.same_contents(existing))
}

/// Return missing or differing source objects in lexicographic key order.
/// Deletions require a separate retention decision and are never planned here.
pub fn plan_replica_copies(
    source: &[ObjectMetadata],
    destination: &[ObjectMetadata],
) -> Result<Vec<ObjectMetadata>, &'static str> {
    let source = index(source)?;
    let destination = index(destination)?;
    // A generation-scoped key is immutable: returning it for a conflicting
    // copy would ask the transport to overwrite an existing object.
    reject_immutable_conflicts(&source, &destination)?;
    let mut changed: Vec<ObjectMetadata> = source
        .into_values()
        .filter(|meta| needs_copy(meta, &destination))
        .cloned()
        .collect();
    changed.sort_unstable_by(|left, right| left.key.cmp(&right.key));
    Ok(changed)
}

// Greedy ordering is by key, with oversized objects skipped. A small batch
// can be found from a progressively expanded prefix without sorting the
// entire inventory. Returning fewer than max_objects requires another page.
fn select_copy_batch(
    ordered: &[&ObjectMetadata],
    max_bytes: u64,
    max_objects: usize,
) -> Vec<ObjectMetadata> {
    let mut remaining = max_bytes;
    let mut selected = Vec::with_capacity(ordered.len().min(max_objects));
    for &object in ordered {
        if selected.len() == max_objects {
            break;
        }
        if object.bytes <= remaining {
            remaining -= object.bytes;
            selected.push(object.clone());
        }
    }
    selected
}

/// Select a bounded batch from the deterministic copy-only reconciliation plan.
/// Oversized objects are skipped so smaller copies can still make progress.
pub fn plan_replica_copy_batch(
    source: &[ObjectMetadata],
    destination: &[ObjectMetadata],
    max_bytes: u64,
    max_objects: usize,
) -> Result<Vec<ObjectMetadata>, &'static str> {
    if max_objects > 65_536 {
        return Err("replica copy batch exceeds object limit");
    }
    // Validate all inventory entries before planning any copies.
    // Sort borrowed metadata, then clone only the objects admitted by
    // the bounded batch rather than the entire changed inventory.
    let source_index = index(source)?;
    let destination_index = index(destination)?;
    reject_immutable_conflicts(&source_index, &destination_index)?;
    if max_objects == 0 {
        return Ok(Vec::new());
    }
    if max_objects == 1 {
        // A scheduler requesting one copy only needs the smallest eligible
        // object key, not a sorted list of the entire changed inventory.
        return Ok(source_index
            .into_values()
            .filter(|meta| meta.bytes <= max_bytes)
            .filter(|meta| needs_copy(meta, &destination_index))
            .min_by(|left, right| left.key.cmp(&right.key))
            .cloned()
            .into_iter()
            .collect());
    }
    let mut needed = source_index
        .into_values()
        .filter(|meta| needs_copy(meta, &destination_index))
        // Remaining budget only decreases during the greedy copy walk.
        // Objects larger than the initial budget cannot enter this batch,
        // so exclude them before allocating and ranking candidates.
        .filter(|meta| meta.bytes <= max_bytes)
        .collect::<Vec<_>>();
    if max_objects.saturating_mul(4) < needed.len() {
        // Select progressively larger lexicographic prefixes. Most tiny
        // batches finish after one linear partition and a short sort.
        // Expanding when the byte budget skips large objects preserves
        // the exact full-plan greedy semantics, including zero-byte keys.
        let mut prefix = max_objects.saturating_mul(2).min(needed.len());
        loop {
            if prefix < needed.len() {
                needed.select_nth_unstable_by(prefix, |a, b| a.key.cmp(&b.key));
            }
            needed[..prefix].sort_unstable_by(|a, b| a.key.cmp(&b.key));
            let selected = select_copy_batch(&needed[..prefix], max_bytes, max_objects);
            if selected.len() == max_objects || prefix == needed.len() {
                return Ok(selected);
            }
            prefix = prefix.saturating_mul(2).min(needed.len());
        }
    }
    // When the whole missing set fits the byte budget, selection only
    // needs the lexicographically earliest max_objects entries. Partition
    // in linear time and sort that prefix, not the entire inventory.
    // Constrained budgets retain the original greedy scan semantics.
    if needed.len() > max_objects
        && needed.iter().try_fold(0u64, |total, object| {
            total.checked_add(object.bytes)
        }).is_some_and(|total| total <= max_bytes)
    {
        needed.select_nth_unstable_by(max_objects, |left, right| left.key.cmp(&right.key));
        needed.truncate(max_objects);
    }
    needed.sort_unstable_by(|left, right| left.key.cmp(&right.key));
    Ok(select_copy_batch(&needed, max_bytes, max_objects))
}

#[cfg(test)]
mod expanding_copy_prefix_tests {
    use super::*;

    #[test]
    fn tiny_batches_match_full_sorted_greedy_plan_at_inventory_scale() {
        let source = (0..8_192usize)
            .rev()
            .map(|index| ObjectMetadata {
                key: format!("part/object_{index:05}"),
                bytes: if index % 17 == 0 { 0 } else { (index % 257) as u64 },
                crc32c: index as u32,
            })
            .collect::<Vec<_>>();
        let destination = source.iter().step_by(7).cloned().collect::<Vec<_>>();
        let full = plan_replica_copies(&source, &destination).unwrap();
        for budget in [0, 1, 23, 100, 1024, u64::MAX] {
            for limit in [2, 3, 8, 31, 127] {
                let mut remaining = budget;
                let expected = full.iter().filter(|object| {
                    if object.bytes <= remaining {
                        remaining -= object.bytes;
                        true
                    } else {
                        false
                    }
                }).take(limit).cloned().collect::<Vec<_>>();
                assert_eq!(
                    plan_replica_copy_batch(&source, &destination, budget, limit).unwrap(),
                    expected, "budget={budget}, limit={limit}",
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
}

#[cfg(test)]
mod copy_batch_tests {
    use super::*;

    #[test]
    fn bounded_batch_matches_full_plan_for_many_objects() {
        let source = (0..500)
            .rev()
            .map(|index| ObjectMetadata {
                key: format!("part/{index:04}.bin"),
                bytes: if index % 7 == 0 { 500 } else { 1 },
                crc32c: index,
            })
            .collect::<Vec<_>>();
        let expected = plan_replica_copies(&source, &[]).unwrap();
        let mut remaining = 12u64;
        let selected = expected
            .into_iter()
            .filter(|object| {
                if object.bytes > remaining { return false; }
                remaining -= object.bytes;
                true
            })
            .take(12)
            .collect::<Vec<_>>();
        assert_eq!(plan_replica_copy_batch(&source, &[], 12, 12).unwrap(), selected);
    }

    #[test]
    fn one_copy_batch_matches_the_sorted_plan_without_a_full_sort() {
        let source = (0..512).rev().map(|index| ObjectMetadata {
            key: format!("part/{index:04}.bin"),
            bytes: 1 + (index % 9) as u64,
            crc32c: index,
        }).collect::<Vec<_>>();
        let destination = source.iter().filter(|object| object.key.ends_with('0'))
            .cloned().collect::<Vec<_>>();
        let all = plan_replica_copies(&source, &destination).unwrap();
        for budget in [0u64, 1, 3, 5, 12] {
            let expected = all.iter().find(|object| object.bytes <= budget).cloned();
            assert_eq!(
                plan_replica_copy_batch(&source, &destination, budget, 1)
                    .unwrap()
                    .into_iter()
                    .next(),
                expected,
            );
        }
        assert!(plan_replica_copy_batch(&source, &destination, u64::MAX, 0)
            .unwrap().is_empty());
        let invalid = [ObjectMetadata { key: "../bad".into(), bytes: 0, crc32c: 0 }];
        assert!(plan_replica_copy_batch(&invalid, &[], 0, 0).is_err());
    }

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

#[cfg(test)]
mod immutable_collision_tests {
    use super::*;

    #[test]
    fn batch_refuses_same_key_with_different_contents() {
        let source = [ObjectMetadata { key: "p/g1/id.bin".into(), bytes: 10, crc32c: 1 }];
        let conflicting = [ObjectMetadata { key: "p/g1/id.bin".into(), bytes: 10, crc32c: 2 }];
        assert!(plan_replica_copy_batch(&source, &conflicting, 100, 10).is_err());
        assert!(plan_replica_copy_batch(&source, &source, 100, 10).unwrap().is_empty());
        assert_eq!(plan_replica_copy_batch(&source, &[], 100, 10).unwrap(), vec![source[0].clone()]);
    }
}

#[cfg(test)]
mod windows_inventory_keys_tests {
    use super::*;

    #[test]
    fn rejects_reserved_device_names_in_any_inventory_path_segment() {
        for key in ["NUL", "p/CON.txt", "a/com1.bin", "a/b/LPT9.log", "prn.csv/part", "AUX"] {
            let value = ObjectMetadata { key: key.to_owned(), bytes: 1, crc32c: 0 };
            assert!(plan_replica_copies(&[value.clone()], &[]).is_err(), "{key}");
            assert!(plan_replica_copy_batch(&[value], &[], 1, 1).is_err(), "{key}");
        }
        for key in ["a/CONTEXT.txt", "x/com10.bin", "z/LPT0.log"] {
            let value = ObjectMetadata { key: key.to_owned(), bytes: 1, crc32c: 0 };
            assert_eq!(plan_replica_copies(&[value.clone()], &[]).unwrap(), vec![value]);
        }
    }
}

#[cfg(test)]
mod inventory_component_budget_tests {
    use super::*;

    #[test]
    fn rejects_oversized_path_components_without_rejecting_long_valid_keys() {
        for key in [
            "x".repeat(129),
            format!("prefix/{}", "a".repeat(129)),
            format!("{}/suffix", "z".repeat(129)),
        ] {
            let meta = ObjectMetadata { key, bytes: 1, crc32c: 0 };
            assert!(plan_replica_copies(&[meta.clone()], &[]).is_err());
            assert!(plan_replica_copy_batch(&[meta], &[], 1, 1).is_err());
        }
        let key = format!("{}/{}", "a".repeat(128), "b".repeat(128));
        let meta = ObjectMetadata { key, bytes: 1, crc32c: 0 };
        assert_eq!(plan_replica_copies(&[meta.clone()], &[]).unwrap(), vec![meta]);
    }
}


#[cfg(test)]
mod unconstrained_prefix_selection_tests {
    use super::*;

    #[test]
    fn bounded_prefix_matches_full_copy_plan_at_inventory_scale() {
        let source = (0..2048usize).rev().map(|index| ObjectMetadata {
            key: format!("part/{index:05}.bin"),
            bytes: (index % 17) as u64,
            crc32c: index as u32,
        }).collect::<Vec<_>>();
        let destination = source.iter().filter(|meta| meta.crc32c % 5 == 0)
            .cloned().collect::<Vec<_>>();
        let all = plan_replica_copies(&source, &destination).unwrap();
        for count in [2, 3, 8, 32, 256, 2048, 4096] {
            let selected = plan_replica_copy_batch(&source, &destination, u64::MAX, count).unwrap();
            assert_eq!(selected, all.iter().take(count).cloned().collect::<Vec<_>>());
        }
    }

    #[test]
    fn tight_budgets_keep_greedy_skips_and_zero_byte_objects() {
        let source = [
            ObjectMetadata { key: "a/x".into(), bytes: 9, crc32c: 0 },
            ObjectMetadata { key: "b/x".into(), bytes: 0, crc32c: 0 },
            ObjectMetadata { key: "c/x".into(), bytes: 2, crc32c: 0 },
            ObjectMetadata { key: "d/x".into(), bytes: 1, crc32c: 0 },
            ObjectMetadata { key: "e/x".into(), bytes: 1, crc32c: 0 },
        ];
        assert_eq!(
            plan_replica_copy_batch(&source, &[], 3, 2).unwrap()
                .iter().map(|meta| meta.key.as_str()).collect::<Vec<_>>(),
            vec!["b/x", "c/x"]
        );
        assert_eq!(
            plan_replica_copy_batch(&source, &[], 0, 3).unwrap()
                .iter().map(|meta| meta.key.as_str()).collect::<Vec<_>>(),
            vec!["b/x"]
        );
        // An enormous missing object must not overflow the prefix sum.
        let huge = [
            ObjectMetadata { key: "a/x".into(), bytes: u64::MAX, crc32c: 1 },
            ObjectMetadata { key: "b/x".into(), bytes: 5, crc32c: 2 },
            ObjectMetadata { key: "c/x".into(), bytes: 0, crc32c: 3 },
        ];
        assert_eq!(
            plan_replica_copy_batch(&huge, &[], 5, 2).unwrap()
                .iter().map(|meta| meta.key.as_str()).collect::<Vec<_>>(),
            vec!["b/x", "c/x"]
        );
    }
}

#[cfg(test)]
mod unaffordable_copy_candidates_tests {
    use super::*;

    #[test]
    fn large_immutable_objects_do_not_hide_later_affordable_copies() {
        let source = (0..8_192)
            .rev()
            .map(|index| ObjectMetadata {
                key: format!("part/file_{index:05}"),
                bytes: if index % 101 == 0 { 0 } else if index % 17 == 0 { 1 } else { 1_000_000 },
                crc32c: index,
            })
            .collect::<Vec<_>>();
        let full = plan_replica_copies(&source, &[]).unwrap();
        for budget in [0, 1, 2, 8, 64] {
            for limit in [2, 3, 5, 16, 64, 257] {
                let mut remaining = budget;
                let expected = full.iter()
                    .filter(|object| {
                        if object.bytes > remaining { return false; }
                        remaining -= object.bytes;
                        true
                    })
                    .take(limit).cloned().collect::<Vec<_>>();
                assert_eq!(
                    plan_replica_copy_batch(&source, &[], budget, limit).unwrap(),
                    expected, "budget={budget} limit={limit}",
                );
            }
        }
    }
}
