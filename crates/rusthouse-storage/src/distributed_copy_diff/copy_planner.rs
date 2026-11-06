//! Replica copy batching and progress pagination.

use super::*;

// Greedy ordering is by key, with oversized objects skipped. A small batch
// can be found from a progressively expanded prefix without sorting the
// entire inventory. Returning fewer than max_objects requires another page.
pub(super) fn select_copy_batch(
    ordered: &[&ObjectMetadata],
    max_bytes: u64,
    max_objects: usize,
) -> Vec<ObjectMetadata> {
    let mut remaining = max_bytes;
    // A small byte budget may select only one object from 65,536 eligible
    // candidates. Grow only as items are accepted, rather than reserving
    // the maximum-sized request up front.
    let mut selected = Vec::with_capacity(ordered.len().min(max_objects).min(64));
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

/// Reuse validated immutable-object inventories across many transfer pages.
///
/// Building the source and destination indexes only once avoids O(inventory)
/// hashing and sorting on every small paged reconciliation request. The source
/// slice must remain frozen while this borrowed planner is in use.
pub struct ReplicaCopyPlanner<'a> {
    missing: Vec<&'a ObjectMetadata>,
}

impl<'a> ReplicaCopyPlanner<'a> {
    /// Reject duplicate keys and immutable-content conflicts before planning.
    pub fn new(
        source: &'a [ObjectMetadata],
        destination: &[ObjectMetadata],
    ) -> Result<Self, &'static str> {
        if source.len() > 65_536 || destination.len() > 65_536 {
            return Err("replica inventory too large");
        }
        reject_portable_key_aliases(source, destination)?;
        let sorted =
            |objects: &[ObjectMetadata]| objects.windows(2).all(|pair| pair[0].key < pair[1].key);
        if sorted(source) && sorted(destination) {
            if source
                .iter()
                .chain(destination)
                .any(|object| !valid_key(&object.key))
            {
                return Err("unsafe or duplicate replica object key");
            }
            let mut missing = Vec::new();
            let mut dst = 0usize;
            for object in source {
                while dst < destination.len() && destination[dst].key.as_str() < object.key.as_str()
                {
                    dst += 1;
                }
                match destination.get(dst) {
                    Some(existing) if existing.key == object.key => {
                        if !object.same_contents(existing) {
                            return Err("replica copy conflicts with an existing immutable object");
                        }
                    }
                    _ => missing.push(object),
                }
            }
            return Ok(Self { missing });
        }
        let source_index = index(source)?;
        let destination_index = index(destination)?;
        reject_immutable_conflicts(&source_index, &destination_index)?;
        let mut missing = source_index
            .into_values()
            .filter(|object| needs_copy(object, &destination_index))
            .collect::<Vec<_>>();
        missing.sort_unstable_by(|left, right| left.key.cmp(&right.key));
        Ok(Self { missing })
    }

    pub fn len(&self) -> usize {
        self.missing.len()
    }

    pub fn is_empty(&self) -> bool {
        self.missing.is_empty()
    }

    /// Resume strictly after the named object key in O(log N), then perform
    /// the same budget-constrained greedy selection as the one-shot planner.
    pub fn batch_after(
        &self,
        after_key: Option<&str>,
        max_bytes: u64,
        max_objects: usize,
    ) -> Result<Vec<ObjectMetadata>, &'static str> {
        if after_key.is_some_and(|key| !valid_key(key)) {
            return Err("invalid replica copy continuation key");
        }
        if max_objects > 65_536 {
            return Err("replica copy batch exceeds object limit");
        }
        let first = after_key.map_or(0, |cursor| {
            self.missing
                .partition_point(|object| object.key.as_str() <= cursor)
        });
        Ok(select_copy_batch(
            &self.missing[first..],
            max_bytes,
            max_objects,
        ))
    }
}

/// A copy-only transfer page that cannot silently skip a missing object.
///
/// next_after is an exclusive cursor for the next request. A complete page
/// has no next cursor, so callers can stop without an extra empty poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaCopyPage {
    pub copies: Vec<ObjectMetadata>,
    pub next_after: Option<String>,
}

impl<'a> ReplicaCopyPlanner<'a> {
    /// Bound output memory and bytes without skipping earlier missing keys.
    ///
    /// Unlike greedy batch selection, a page stops at the first object that
    /// does not fit its remaining budget. A first object larger than the
    /// entire budget is an error, not an endlessly repeating empty page.
    pub fn page_after(
        &self,
        after_key: Option<&str>,
        max_bytes: u64,
        max_objects: usize,
    ) -> Result<ReplicaCopyPage, &'static str> {
        if after_key.is_some_and(|key| !valid_key(key)) {
            return Err("invalid replica copy continuation key");
        }
        if max_objects == 0 || max_objects > 65_536 {
            return Err("invalid lossless replica copy page limit");
        }
        let start = after_key.map_or(0, |cursor| {
            self.missing
                .partition_point(|item| item.key.as_str() <= cursor)
        });
        let mut remaining_bytes = max_bytes;
        // Do not allocate capacity for an entire inventory before checking
        // how many immutable objects fit the caller's byte budget.
        let mut copies = Vec::with_capacity((self.missing.len() - start).min(max_objects).min(64));
        for &object in &self.missing[start..] {
            if copies.len() == max_objects {
                break;
            }
            if object.bytes > remaining_bytes {
                if copies.is_empty() {
                    return Err("next immutable object exceeds replica copy byte budget");
                }
                break;
            }
            remaining_bytes -= object.bytes;
            copies.push(object.clone());
        }
        let next_after = (start + copies.len() < self.missing.len())
            .then(|| copies.last().expect("nonempty incomplete page").key.clone());
        Ok(ReplicaCopyPage { copies, next_after })
    }
}

/// Build a complete, losslessly pageable replica-copy plan.
///
/// Validate every source and destination key before returning any copy work.
/// Unlike the greedy scheduler, this API guarantees that following each
/// returned cursor cannot skip a missing object because of a byte budget.
pub fn plan_replica_copy_page(
    source: &[ObjectMetadata],
    destination: &[ObjectMetadata],
    after_key: Option<&str>,
    max_bytes: u64,
    max_objects: usize,
) -> Result<ReplicaCopyPage, &'static str> {
    ReplicaCopyPlanner::new(source, destination)?.page_after(after_key, max_bytes, max_objects)
}

/// Select a bounded batch from the deterministic copy-only reconciliation plan.
/// Oversized objects are skipped so smaller copies can still make progress.
pub fn plan_replica_copy_batch(
    source: &[ObjectMetadata],
    destination: &[ObjectMetadata],
    max_bytes: u64,
    max_objects: usize,
) -> Result<Vec<ObjectMetadata>, &'static str> {
    plan_replica_copy_batch_after(source, destination, None, max_bytes, max_objects)
}

/// Page a lexicographic, immutable-object copy plan after an exclusive key.
/// Cursor filtering happens before candidate ranking, without materializing
/// the whole sorted copy inventory on each transfer batch.
pub fn plan_replica_copy_batch_after(
    source: &[ObjectMetadata],
    destination: &[ObjectMetadata],
    after_key: Option<&str>,
    max_bytes: u64,
    max_objects: usize,
) -> Result<Vec<ObjectMetadata>, &'static str> {
    if after_key.is_some_and(|key| !valid_key(key)) {
        return Err("invalid replica copy continuation key");
    }
    if max_objects > 65_536 {
        return Err("replica copy batch exceeds object limit");
    }
    reject_portable_key_aliases(source, destination)?;
    if source.len() <= 8 && destination.len() <= 8 {
        // Apply the same strict inventory and immutable conflict validation,
        // including when the continuation cursor leaves no eligible objects.
        validate_tiny_inventory(source)?;
        validate_tiny_inventory(destination)?;
        for object in source {
            if let Some(existing) = destination.iter().find(|item| item.key == object.key) {
                if !object.same_contents(existing) {
                    return Err("replica copy conflicts with an existing immutable object");
                }
            }
        }
        let mut needed = source
            .iter()
            .filter(|item| after_key.is_none_or(|cursor| item.key.as_str() > cursor))
            .filter(|item| !destination.iter().any(|existing| existing.key == item.key))
            .collect::<Vec<_>>();
        needed.sort_unstable_by(|left, right| left.key.cmp(&right.key));
        return Ok(select_copy_batch(&needed, max_bytes, max_objects));
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
            .filter(|meta| after_key.is_none_or(|cursor| meta.key.as_str() > cursor))
            .filter(|meta| needs_copy(meta, &destination_index))
            .min_by(|left, right| left.key.cmp(&right.key))
            .cloned()
            .into_iter()
            .collect());
    }
    if max_objects == 2 {
        // Two sequential minima reproduce the sorted greedy copy plan.
        // Scan borrowed inventory values twice rather than allocating and
        // sorting every missing object on a small transfer poll.
        let first = source_index
            .values()
            .copied()
            .filter(|meta| {
                meta.bytes <= max_bytes
                    && after_key.is_none_or(|cursor| meta.key.as_str() > cursor)
                    && needs_copy(meta, &destination_index)
            })
            .min_by(|left, right| left.key.cmp(&right.key));
        let Some(first) = first else {
            return Ok(Vec::new());
        };
        let remaining = max_bytes - first.bytes;
        let second = source_index
            .values()
            .copied()
            .filter(|meta| {
                meta.key != first.key
                    && after_key.is_none_or(|cursor| meta.key.as_str() > cursor)
                    && meta.bytes <= remaining
                    && needs_copy(meta, &destination_index)
            })
            .min_by(|left, right| left.key.cmp(&right.key));
        let mut selected = Vec::with_capacity(2);
        selected.push(first.clone());
        if let Some(second) = second {
            selected.push(second.clone());
        }
        return Ok(selected);
    }
    let mut needed = source_index
        .into_values()
        .filter(|meta| needs_copy(meta, &destination_index))
        .filter(|meta| after_key.is_none_or(|cursor| meta.key.as_str() > cursor))
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
        && needed
            .iter()
            .try_fold(0u64, |total, object| total.checked_add(object.bytes))
            .is_some_and(|total| total <= max_bytes)
    {
        needed.select_nth_unstable_by(max_objects, |left, right| left.key.cmp(&right.key));
        needed.truncate(max_objects);
    }
    needed.sort_unstable_by(|left, right| left.key.cmp(&right.key));
    Ok(select_copy_batch(&needed, max_bytes, max_objects))
}
