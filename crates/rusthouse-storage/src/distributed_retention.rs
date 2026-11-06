//! Conservative generation-retention planning; no files are deleted.

#[cfg(test)]
#[path = "distributed_retention_tests/age_retention_sorted_lookup_tests.rs"]
mod age_retention_sorted_lookup_tests;

use std::borrow::Cow;

const MAX_RETENTION_GENERATIONS: usize = 16_384;

fn ordered_generation_sizes(generations: &[(u64, u64)]) -> Cow<'_, [(u64, u64)]> {
    if generations.windows(2).all(|pair| pair[0].0 <= pair[1].0) {
        Cow::Borrowed(generations)
    } else {
        let mut ordered = generations.to_vec();
        ordered.sort_unstable_by_key(|&(generation, _)| generation);
        Cow::Owned(ordered)
    }
}

fn validated_generations(generations: &[u64], active: u64) -> Result<Cow<'_, [u64]>, &'static str> {
    // Snapshot inventory is often already chronological. Borrow it without
    // copying or sorting; retain the old sorting path for unordered inputs.
    let sorted_generations = if generations.windows(2).all(|pair| pair[0] < pair[1]) {
        Cow::Borrowed(generations)
    } else {
        let mut owned = generations.to_vec();
        owned.sort_unstable();
        Cow::Owned(owned)
    };
    if sorted_generations
        .first()
        .is_none_or(|generation| *generation == 0)
        || sorted_generations.windows(2).any(|pair| pair[0] == pair[1])
        || sorted_generations.binary_search(&active).is_err()
    {
        return Err("duplicate, missing, or invalid generation");
    }
    Ok(sorted_generations)
}

fn known_generation(generations: &[u64], generation: u64) -> bool {
    generations.binary_search(&generation).is_ok()
}

fn planned_retention<'a>(
    generations: &'a [u64],
    active: u64,
    keep_latest: usize,
) -> Result<(Vec<u64>, Cow<'a, [u64]>), &'static str> {
    if generations.is_empty()
        || generations.len() > MAX_RETENTION_GENERATIONS
        || keep_latest == 0
        || active == 0
    {
        return Err("invalid generation retention configuration");
    }
    let sorted_generations = validated_generations(generations, active)?;
    let candidate_count = sorted_generations.len().saturating_sub(keep_latest);
    let candidates = sorted_generations
        .iter()
        .take(candidate_count)
        .copied()
        .filter(|&generation| generation != active)
        .collect();
    Ok((candidates, sorted_generations))
}

/// List old generations eligible for review by a caller with exclusive ownership.
/// Always preserve the active generation and the newest keep_latest generations.
pub fn retention_candidates(
    generations: &[u64],
    active: u64,
    keep_latest: usize,
) -> Result<Vec<u64>, &'static str> {
    planned_retention(generations, active, keep_latest).map(|(candidates, _)| candidates)
}

#[cfg(test)]
#[path = "distributed_retention_tests/tests.rs"]
mod tests;

/// Plan retention while protecting snapshots pinned by readers, replicas or backups.
/// This only lists candidates; it never deletes generation files.
pub fn retention_candidates_with_pins(
    generations: &[u64],
    active: u64,
    keep_latest: usize,
    pinned: &[u64],
) -> Result<Vec<u64>, &'static str> {
    if pinned.len() > MAX_RETENTION_GENERATIONS {
        return Err("too many pinned generations");
    }
    let (candidates, known) = planned_retention(generations, active, keep_latest)?;
    if pinned.is_empty() {
        // Do not hash every candidate when no snapshot has a reader pin.
        return Ok(candidates);
    }
    if let [pin] = pinned {
        if !known_generation(&known, *pin) {
            return Err("duplicate or unknown pinned generation");
        }
        return Ok(candidates
            .into_iter()
            .filter(|generation| generation != pin)
            .collect());
    }
    // Up to four integer comparisons per candidate are cheaper than building
    // and probing a hash table, even for a full 16K-generation inventory.
    if pinned.len() <= 4 {
        for (index, &generation) in pinned.iter().enumerate() {
            if pinned[..index].contains(&generation) || !known_generation(&known, generation) {
                return Err("duplicate or unknown pinned generation");
            }
        }
        return Ok(candidates
            .into_iter()
            .filter(|generation| !pinned.contains(generation))
            .collect());
    }
    // Reader/replica pin sets are often supplied in chronological order.
    // Merge them with the already sorted candidate list in linear time,
    // with no temporary HashSet even for thousands of generations.
    if pinned.windows(2).all(|pair| pair[0] < pair[1]) {
        // Both inventories are sorted. Confirm every pin is a known
        // generation with one monotonic merge instead of P binary searches.
        let mut known_cursor = 0usize;
        for &pin in pinned {
            while known
                .get(known_cursor)
                .is_some_and(|&generation| generation < pin)
            {
                known_cursor += 1;
            }
            if known.get(known_cursor) != Some(&pin) {
                return Err("duplicate or unknown pinned generation");
            }
        }
        let mut cursor = 0usize;
        return Ok(candidates
            .into_iter()
            .filter(|generation| {
                while pinned.get(cursor).is_some_and(|&pin| pin < *generation) {
                    cursor += 1;
                }
                pinned.get(cursor) != Some(generation)
            })
            .collect());
    }
    let mut pins = std::collections::HashSet::with_capacity(pinned.len());
    for &generation in pinned {
        if !pins.insert(generation) || !known_generation(&known, generation) {
            return Err("duplicate or unknown pinned generation");
        }
    }
    Ok(candidates
        .into_iter()
        .filter(|generation| !pins.contains(generation))
        .collect())
}

#[cfg(test)]
#[path = "distributed_retention_tests/pinned_retention_tests.rs"]
mod pinned_retention_tests;

/// Retain any generation at or above the oldest generation a replica needs.
/// The caller must supply a verified replica catch-up floor.
pub fn retention_candidates_below_replica_floor(
    generations: &[u64],
    active: u64,
    keep_latest: usize,
    oldest_needed: u64,
) -> Result<Vec<u64>, &'static str> {
    if oldest_needed == 0 {
        return Err("invalid replica retention floor");
    }
    Ok(retention_candidates(generations, active, keep_latest)?
        .into_iter()
        .filter(|&generation| generation < oldest_needed)
        .collect())
}

#[cfg(test)]
#[path = "distributed_retention_tests/replica_floor_tests.rs"]
mod replica_floor_tests;

/// Select oldest eligible generations for deletion without exceeding a byte budget.
/// This plans only; the caller must coordinate readers and verify deletions.
pub fn retention_candidates_with_byte_budget(
    generations: &[(u64, u64)],
    active: u64,
    keep_latest: usize,
    pinned: &[u64],
    byte_budget: u64,
) -> Result<(Vec<u64>, u64), &'static str> {
    if generations.is_empty() || generations.len() > MAX_RETENTION_GENERATIONS {
        return Err("invalid generation retention inventory");
    }
    if keep_latest == 0 || active == 0 || pinned.len() > MAX_RETENTION_GENERATIONS {
        return Err("invalid generation retention configuration");
    }
    // Sort generation sizes together. The previous path sorted an ID copy,
    // materialized eligible IDs, and then built a second hash table just
    // to look up each candidate's byte cost.
    let ordered = ordered_generation_sizes(generations);
    if ordered[0].0 == 0
        || ordered.windows(2).any(|pair| pair[0].0 == pair[1].0)
        || ordered
            .binary_search_by_key(&active, |&(id, _)| id)
            .is_err()
    {
        return Err("duplicate, missing, or invalid generation");
    }
    // Validate every pin even if no generation can be reclaimed. Strictly
    // sorted pins can be merged with oldest-first retention candidates,
    // avoiding a HashSet while still rejecting duplicates and unknown IDs.
    let sorted_pins = pinned.len() > 4 && pinned.windows(2).all(|pair| pair[0] < pair[1]);
    let mut pin_set = (pinned.len() > 4 && !sorted_pins)
        .then(|| std::collections::HashSet::with_capacity(pinned.len()));
    for (index, &generation) in pinned.iter().enumerate() {
        let duplicate = if sorted_pins {
            false
        } else if let Some(seen) = pin_set.as_mut() {
            !seen.insert(generation)
        } else {
            pinned[..index].contains(&generation)
        };
        if duplicate
            || ordered
                .binary_search_by_key(&generation, |&(id, _)| id)
                .is_err()
        {
            return Err("duplicate or unknown pinned generation");
        }
    }
    let candidates = ordered.len().saturating_sub(keep_latest);
    let mut selected = Vec::new();
    let mut total = 0u64;
    let mut pin_cursor = 0usize;
    for &(generation, bytes) in ordered.iter().take(candidates) {
        if sorted_pins {
            while pinned.get(pin_cursor).is_some_and(|&pin| pin < generation) {
                pin_cursor += 1;
            }
        }
        let is_pinned = if sorted_pins {
            pinned.get(pin_cursor) == Some(&generation)
        } else {
            pin_set.as_ref().map_or_else(
                || pinned.contains(&generation),
                |pins| pins.contains(&generation),
            )
        };
        if generation == active || is_pinned {
            continue;
        }
        if bytes <= byte_budget - total {
            total += bytes;
            selected.push(generation);
        }
    }
    Ok((selected, total))
}

#[cfg(test)]
#[path = "distributed_retention_tests/fused_budgeted_retention_tests.rs"]
mod fused_budgeted_retention_tests;

#[cfg(test)]
#[path = "distributed_retention_tests/budgeted_retention_tests.rs"]
mod budgeted_retention_tests;

#[cfg(test)]
#[path = "distributed_retention_tests/fully_retained_budget_tests.rs"]
mod fully_retained_budget_tests;

#[cfg(test)]
#[path = "distributed_retention_tests/small_pin_retention_tests.rs"]
mod small_pin_retention_tests;

#[cfg(test)]
#[path = "distributed_retention_tests/singleton_retention_budget_tests.rs"]
mod singleton_retention_budget_tests;

#[cfg(test)]
#[path = "distributed_retention_tests/empty_generation_validation_tests.rs"]
mod empty_generation_validation_tests;

/**
 * Plan retention for multiple catching-up replicas and pinned readers.
 *
 * A generation is reclaimable only if it is older than every replica's
 * verified catch-up floor, not pinned, not active and outside keep_latest.
 * This is a plan only. It performs no filesystem deletions.
 */
pub fn retention_candidates_for_replica_floors(
    generations: &[u64],
    active: u64,
    keep_latest: usize,
    pinned: &[u64],
    oldest_needed_per_replica: &[u64],
) -> Result<Vec<u64>, &'static str> {
    if oldest_needed_per_replica.is_empty()
        || oldest_needed_per_replica.len() > 64
        || oldest_needed_per_replica.contains(&0)
    {
        return Err("invalid replica retention floors");
    }
    // The slowest replica determines what can be reclaimed. Compute the
    // pinned-aware retention plan once instead of intersecting N allocations.
    let oldest_needed = *oldest_needed_per_replica
        .iter()
        .min()
        .expect("nonempty floors validated");
    Ok(
        retention_candidates_with_pins(generations, active, keep_latest, pinned)?
            .into_iter()
            .filter(|&generation| generation < oldest_needed)
            .collect(),
    )
}

#[cfg(test)]
#[path = "distributed_retention_tests/multiple_replica_floor_tests.rs"]
mod multiple_replica_floor_tests;

#[cfg(test)]
#[path = "distributed_retention_tests/borrowed_sorted_retention_tests.rs"]
mod borrowed_sorted_retention_tests;

/// Bound one generation-retention pass by reader pins, the slowest replica's
/// catch-up floor, and an exact total byte budget. This is a deletion plan,
/// not authorization to remove any generation from disk.
pub fn retention_candidates_with_replica_floors_and_budget(
    generations: &[(u64, u64)],
    active: u64,
    keep_latest: usize,
    pinned: &[u64],
    replica_floors: &[u64],
    byte_budget: u64,
) -> Result<(Vec<u64>, u64), &'static str> {
    if generations.is_empty()
        || generations.len() > MAX_RETENTION_GENERATIONS
        || keep_latest == 0
        || active == 0
        || pinned.len() > MAX_RETENTION_GENERATIONS
        || replica_floors.is_empty()
        || replica_floors.len() > 64
        || replica_floors.contains(&0)
    {
        return Err("invalid replica-aware retention configuration");
    }
    let floor = *replica_floors
        .iter()
        .min()
        .expect("nonempty floors checked");
    // Most generation inventories are chronological. Reuse their existing
    // order instead of allocating and sorting up to 16K entries per plan.
    let ordered = ordered_generation_sizes(generations);
    if ordered[0].0 == 0
        || ordered.windows(2).any(|pair| pair[0].0 == pair[1].0)
        || ordered
            .binary_search_by_key(&active, |&(id, _)| id)
            .is_err()
    {
        return Err("duplicate, missing, or invalid generation");
    }
    // Validate pins even when byte_budget is zero. A stale pin must not be
    // silently ignored on a later deletion attempt. Chronologically ordered
    // reader pins are common; merge them with the ordered inventory in O(G+P)
    // time without allocating a large hash table or performing P binary searches.
    let sorted_pins = pinned.len() > 4 && pinned.windows(2).all(|pair| pair[0] < pair[1]);
    let mut pins = (pinned.len() > 4 && !sorted_pins)
        .then(|| std::collections::HashSet::with_capacity(pinned.len()));
    if sorted_pins {
        let mut known_cursor = 0usize;
        for &id in pinned {
            while ordered
                .get(known_cursor)
                .is_some_and(|&(known, _)| known < id)
            {
                known_cursor += 1;
            }
            if ordered
                .get(known_cursor)
                .is_none_or(|&(known, _)| known != id)
            {
                return Err("duplicate or unknown pinned generation");
            }
        }
    } else {
        for (index, &id) in pinned.iter().enumerate() {
            let duplicate = if let Some(seen) = pins.as_mut() {
                !seen.insert(id)
            } else {
                pinned[..index].contains(&id)
            };
            if duplicate || ordered.binary_search_by_key(&id, |&(key, _)| key).is_err() {
                return Err("duplicate or unknown pinned generation");
            }
        }
    }
    let eligible = ordered.len().saturating_sub(keep_latest);
    let mut selected = Vec::new();
    let mut consumed = 0u64;
    let mut pin_cursor = 0usize;
    for &(id, size) in ordered.iter().take(eligible) {
        let is_pinned = if sorted_pins {
            while pinned.get(pin_cursor).is_some_and(|&pin| pin < id) {
                pin_cursor += 1;
            }
            pinned.get(pin_cursor) == Some(&id)
        } else {
            pins.as_ref()
                .map_or_else(|| pinned.contains(&id), |seen| seen.contains(&id))
        };
        if id == active || id >= floor || is_pinned {
            continue;
        }
        // Subtraction cannot underflow because consumed never exceeds budget.
        if size <= byte_budget - consumed {
            consumed += size;
            selected.push(id);
        }
    }
    Ok((selected, consumed))
}

#[cfg(test)]
#[path = "distributed_retention_tests/combined_replica_retention_tests.rs"]
mod combined_replica_retention_tests;

#[cfg(test)]
#[path = "distributed_retention_tests/large_inventory_few_pins_tests.rs"]
mod large_inventory_few_pins_tests;

#[cfg(test)]
#[path = "distributed_retention_tests/borrowed_budgeted_retention_tests.rs"]
mod borrowed_budgeted_retention_tests;

/// Plan time-bounded generation reclamation without bypassing catch-up floors.
///
/// Each timestamp must come from verified immutable snapshot metadata.
/// No filesystem operations are performed by this planner.
pub fn retention_candidates_with_min_age(
    generations: &[(u64, u64)],
    active: u64,
    keep_latest: usize,
    pinned: &[u64],
    oldest_needed_per_replica: &[u64],
    now_seconds: u64,
    minimum_age_seconds: u64,
) -> Result<Vec<u64>, &'static str> {
    if generations.is_empty() || generations.len() > MAX_RETENTION_GENERATIONS {
        return Err("invalid generation retention inventory");
    }
    // Snapshot inventories are normally chronological. Borrow their sorted
    // generation/timestamp pairs rather than building a hash table for every
    // retention poll. An unordered inventory still gets one owned sort.
    let ordered = ordered_generation_sizes(generations);
    let mut ids = Vec::with_capacity(ordered.len());
    for (index, &(generation, created_at)) in ordered.iter().enumerate() {
        if created_at > now_seconds {
            return Err("generation timestamp is in the future");
        }
        if index > 0 && ordered[index - 1].0 == generation {
            return Err("duplicate generation timestamp");
        }
        ids.push(generation);
    }
    let eligible = retention_candidates_for_replica_floors(
        &ids,
        active,
        keep_latest,
        pinned,
        oldest_needed_per_replica,
    )?;
    Ok(eligible
        .into_iter()
        .filter(|generation| {
            let index = ids
                .binary_search(generation)
                .expect("eligible generation was validated against sorted inventory");
            now_seconds - ordered[index].1 >= minimum_age_seconds
        })
        .collect())
}

#[cfg(test)]
#[path = "distributed_retention_tests/min_age_sorted_inventory_tests.rs"]
mod min_age_sorted_inventory_tests;

#[cfg(test)]
#[path = "distributed_retention_tests/age_and_replica_floor_retention_tests.rs"]
mod age_and_replica_floor_retention_tests;

#[cfg(test)]
#[path = "distributed_retention_tests/sorted_replica_aware_inventory_tests.rs"]
mod sorted_replica_aware_inventory_tests;

#[cfg(test)]
#[path = "distributed_retention_tests/ordered_replica_pin_merge_tests.rs"]
mod ordered_replica_pin_merge_tests;

#[cfg(test)]
#[path = "distributed_retention_tests/linear_pin_membership_tests.rs"]
mod linear_pin_membership_tests;

#[cfg(test)]
#[path = "distributed_retention_tests/linear_sorted_pin_retention_tests.rs"]
mod linear_sorted_pin_retention_tests;

/// Generation and its optional direct predecessor in a complete offline history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationParent {
    pub generation: u64,
    pub parent: Option<u64>,
}

/// Plan deletion candidates while retaining the ancestor closure of every pin.
///
/// The caller supplies a complete, immutable generation-parent inventory.
/// Unknown parents, nonmonotonic edges, duplicate generations and missing pins
/// fail closed. Results are sorted by generation. No filesystem changes occur.
pub fn retention_candidates_with_ancestry(
    history: &[GenerationParent],
    active: u64,
    pinned: &[u64],
    keep_newest: usize,
) -> Result<Vec<u64>, &'static str> {
    if history.is_empty() || history.len() > 16_384 || pinned.len() > 16_384 {
        return Err("invalid snapshot ancestry inventory size");
    }
    let mut ordered = history.to_vec();
    ordered.sort_unstable_by_key(|entry| entry.generation);
    for (index, entry) in ordered.iter().enumerate() {
        if entry.generation == 0
            || (index > 0 && ordered[index - 1].generation == entry.generation)
            || entry
                .parent
                .is_some_and(|parent| parent == 0 || parent >= entry.generation)
        {
            return Err("invalid or duplicate snapshot ancestry");
        }
        if let Some(parent) = entry.parent {
            if ordered
                .binary_search_by_key(&parent, |candidate| candidate.generation)
                .is_err()
            {
                return Err("snapshot ancestry references a missing parent");
            }
        }
    }
    let index_of = |generation: u64| {
        ordered
            .binary_search_by_key(&generation, |entry| entry.generation)
            .map_err(|_| "snapshot retention anchor is missing")
    };
    let active_index = index_of(active)?;
    let mut anchors = Vec::with_capacity(pinned.len() + keep_newest.min(ordered.len()) + 1);
    anchors.push(active_index);
    for &pin in pinned {
        anchors.push(index_of(pin)?);
    }
    let newest_start = ordered.len().saturating_sub(keep_newest);
    anchors.extend(newest_start..ordered.len());

    // Each generation is followed at most once: marking an already retained
    // generation means its entire parent chain was marked earlier.
    let mut retained = vec![false; ordered.len()];
    for anchor in anchors {
        let mut current = Some(anchor);
        while let Some(index) = current {
            if retained[index] {
                break;
            }
            retained[index] = true;
            current = ordered[index].parent.map(index_of).transpose()?;
        }
    }
    Ok(ordered
        .iter()
        .zip(retained)
        .filter_map(|(entry, keep)| (!keep).then_some(entry.generation))
        .collect())
}

#[cfg(test)]
#[path = "distributed_retention_tests/ancestry_retention_tests.rs"]
mod ancestry_retention_tests;
