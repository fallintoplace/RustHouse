//! Conservative generation-retention planning; no files are deleted.

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
mod tests {
    use super::*;

    #[test]
    fn preserves_current_and_recent_generations() {
        assert_eq!(retention_candidates(&[4, 1, 3, 2], 2, 2).unwrap(), vec![1]);
        assert!(retention_candidates(&[1, 2], 2, 5).unwrap().is_empty());
    }

    #[test]
    fn rejects_unknown_or_duplicate_active_generation() {
        assert!(retention_candidates(&[1, 1, 2], 1, 1).is_err());
        assert!(retention_candidates(&[1, 2], 3, 1).is_err());
        assert!(retention_candidates(&[1, 2], 1, 0).is_err());
    }
    #[test]
    fn protects_active_generation_even_when_it_is_oldest() {
        assert_eq!(
            retention_candidates(&[5, 3, 1, 4, 2], 1, 2).unwrap(),
            vec![2, 3]
        );
        assert_eq!(
            retention_candidates(&[5, 3, 1, 4, 2], 5, 2).unwrap(),
            vec![1, 2, 3]
        );
        assert!(
            retention_candidates(&[1, 2, 3], 1, usize::MAX)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn unsorted_generations_still_produce_sorted_retention_candidates() {
        assert_eq!(
            retention_candidates(&[12, 5, 8, 3, 20, 1], 5, 2).unwrap(),
            vec![1, 3, 8]
        );
        assert!(retention_candidates(&[3, 1, 3], 1, 1).is_err());
    }
}

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
        if pinned.iter().any(|&pin| !known_generation(&known, pin)) {
            return Err("duplicate or unknown pinned generation");
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
mod pinned_retention_tests {
    use super::*;

    #[test]
    fn single_pin_avoids_indexing_and_preserves_retention_order() {
        let generations = (1..=32_u64).rev().collect::<Vec<_>>();
        let candidates = retention_candidates_with_pins(&generations, 32, 2, &[3]).unwrap();
        assert_eq!(candidates.first(), Some(&1));
        assert!(!candidates.contains(&3));
        assert_eq!(candidates.last(), Some(&30));
        assert!(retention_candidates_with_pins(&generations, 32, 2, &[99]).is_err());
    }

    #[test]
    fn preserves_backup_pins_and_active_generation() {
        assert_eq!(
            retention_candidates_with_pins(&[1, 2, 3, 4, 5], 5, 2, &[1, 3]).unwrap(),
            vec![2]
        );
        assert_eq!(
            retention_candidates_with_pins(&[1, 2, 3], 1, 1, &[2]).unwrap(),
            Vec::<u64>::new()
        );
    }

    #[test]
    fn no_pins_use_the_same_retention_plan_as_unpinned_reads() {
        let generations = (1..=16_384_u64).rev().collect::<Vec<_>>();
        for active in [1u64, 8_192, 16_384] {
            let expected = retention_candidates(&generations, active, 128).unwrap();
            assert_eq!(
                retention_candidates_with_pins(&generations, active, 128, &[]).unwrap(),
                expected,
            );
        }
        assert!(retention_candidates_with_pins(&[1, 1], 1, 1, &[]).is_err());
    }

    #[test]
    fn rejects_stale_and_duplicate_pins() {
        assert!(retention_candidates_with_pins(&[1, 2, 3], 3, 1, &[4]).is_err());
        assert!(retention_candidates_with_pins(&[1, 2, 3], 3, 1, &[1, 1]).is_err());
        assert!(retention_candidates_with_pins(&[1, 1], 1, 1, &[]).is_err());
    }
}

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
mod replica_floor_tests {
    use super::*;

    #[test]
    fn does_not_reclaim_generations_a_replica_still_needs() {
        assert_eq!(
            retention_candidates_below_replica_floor(&[1, 2, 3, 4, 5], 5, 2, 3).unwrap(),
            vec![1, 2]
        );
        assert_eq!(
            retention_candidates_below_replica_floor(&[1, 2, 3, 4], 4, 1, 1).unwrap(),
            Vec::<u64>::new()
        );
        assert!(retention_candidates_below_replica_floor(&[1, 2], 2, 1, 0).is_err());
    }
}

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
mod fused_budgeted_retention_tests {
    use super::*;

    #[test]
    fn fused_budget_matches_existing_pinned_eligibility_at_scale() {
        let generations = (1..=4096u64)
            .rev()
            .map(|id| (id, (id * 31) % 997))
            .collect::<Vec<_>>();
        let ids = generations.iter().map(|&(id, _)| id).collect::<Vec<_>>();
        let pinned = [3, 11, 121, 2048, 4095];
        let eligible = retention_candidates_with_pins(&ids, 4096, 100, &pinned).unwrap();
        let mut sizes = generations.clone();
        sizes.sort_unstable_by_key(|&(id, _)| id);
        for budget in [0u64, 10, 512, 20_000, u64::MAX] {
            let mut used = 0u64;
            let mut expected = Vec::new();
            for &id in &eligible {
                let bytes = sizes
                    .binary_search_by_key(&id, |&(generation, _)| generation)
                    .map(|index| sizes[index].1)
                    .unwrap();
                if bytes <= budget - used {
                    used += bytes;
                    expected.push(id);
                }
            }
            assert_eq!(
                retention_candidates_with_byte_budget(&generations, 4096, 100, &pinned, budget,)
                    .unwrap(),
                (expected, used),
            );
        }
    }

    #[test]
    fn rejects_invalid_pins_even_when_every_generation_is_kept() {
        assert!(
            retention_candidates_with_byte_budget(&[(1, 4), (2, 8)], 2, 2, &[1, 1], 100,).is_err()
        );
        assert!(
            retention_candidates_with_byte_budget(&[(1, 4), (2, 8)], 2, 2, &[3], 100,).is_err()
        );
    }
}

#[cfg(test)]
mod budgeted_retention_tests {
    use super::*;

    #[test]
    fn protects_active_latest_and_pinned_generations_under_budget() {
        let generations = [(5, 1u64), (1, 7), (2, 12), (3, 3), (4, 2)];
        assert_eq!(
            retention_candidates_with_byte_budget(&generations, 5, 1, &[3], 9).unwrap(),
            (vec![1, 4], 9)
        );
        assert_eq!(
            retention_candidates_with_byte_budget(&generations, 5, 1, &[3], 21).unwrap(),
            (vec![1, 2, 4], 21)
        );
        assert!(retention_candidates_with_byte_budget(&generations, 5, 1, &[99], 20).is_err());
    }
}

#[cfg(test)]
mod fully_retained_budget_tests {
    use super::*;

    #[test]
    fn fully_retained_generations_have_no_budgeted_work() {
        let generations = (1..=16_384_u64).map(|id| (id, 4)).collect::<Vec<_>>();
        assert_eq!(
            retention_candidates_with_byte_budget(
                &generations,
                16_384,
                generations.len(),
                &[],
                u64::MAX,
            )
            .unwrap(),
            (Vec::<u64>::new(), 0),
        );
        assert!(
            retention_candidates_with_byte_budget(&[(1, 4), (1, 8)], 1, 2, &[], u64::MAX,).is_err()
        );
    }
}

#[cfg(test)]
mod small_pin_retention_tests {
    use super::*;

    #[test]
    fn low_cardinality_pins_preserve_large_path_semantics() {
        let generations = (1..=130u64).collect::<Vec<_>>();
        for pins in [
            vec![],
            vec![1],
            vec![1, 3],
            vec![1, 3, 5, 7],
            vec![1, 3, 5, 7, 9],
        ] {
            let small = retention_candidates_with_pins(&generations, 130, 2, &pins).unwrap();
            assert_eq!(
                small,
                (1..=128u64)
                    .filter(|id| !pins.contains(id))
                    .collect::<Vec<_>>()
            );
        }
        assert!(retention_candidates_with_pins(&generations, 130, 2, &[1, 1]).is_err());
        assert!(retention_candidates_with_pins(&generations, 130, 2, &[131]).is_err());
    }
}

#[cfg(test)]
mod singleton_retention_budget_tests {
    use super::*;

    #[test]
    fn one_eligible_generation_respects_exact_byte_budget() {
        let sizes = [(4, 100u64), (1, 25), (3, 10), (2, 5)];
        assert_eq!(
            retention_candidates_with_byte_budget(&sizes, 4, 3, &[], 24).unwrap(),
            (vec![], 0)
        );
        assert_eq!(
            retention_candidates_with_byte_budget(&sizes, 4, 3, &[], 25).unwrap(),
            (vec![1], 25)
        );
        assert_eq!(
            retention_candidates_with_byte_budget(&sizes, 4, 3, &[], u64::MAX).unwrap(),
            (vec![1], 25)
        );
        assert_eq!(
            retention_candidates_with_byte_budget(&sizes, 4, 3, &[1], u64::MAX).unwrap(),
            (vec![], 0)
        );
        assert!(retention_candidates_with_byte_budget(&[(1, 1), (1, 2)], 1, 1, &[], 100).is_err());
    }
}

#[cfg(test)]
mod empty_generation_validation_tests {
    use super::*;

    #[test]
    fn rejects_empty_generation_history_without_panicking() {
        assert!(validated_generations(&[], 1).is_err());
    }
}

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
mod multiple_replica_floor_tests {
    use super::*;

    #[test]
    fn slowest_replica_and_reader_pins_are_jointly_protected() {
        let generations = [1, 2, 3, 4, 5, 6, 7];
        let selected =
            retention_candidates_for_replica_floors(&generations, 7, 2, &[2], &[5, 3, 6]).unwrap();
        assert_eq!(selected, vec![1]);
        assert_eq!(
            retention_candidates_for_replica_floors(&generations, 7, 2, &[2], &[6, 5, 3]).unwrap(),
            selected,
        );
        assert_eq!(
            retention_candidates_for_replica_floors(&generations, 7, 2, &[], &[3]).unwrap(),
            vec![1, 2],
        );
        assert_eq!(
            retention_candidates_for_replica_floors(&generations, 7, 2, &[2], &[1]).unwrap(),
            Vec::<u64>::new(),
        );
    }

    #[test]
    fn multiple_floors_match_intersection_of_all_single_replica_plans() {
        let generations = (1..=256u64).rev().collect::<Vec<_>>();
        let pins = [2, 9, 17, 53, 120];
        for active in [1u64, 53, 256] {
            for keep in [1, 2, 8, 256, usize::MAX] {
                for floors in [&[1u64, 99, 234][..], &[2u64, 100, 255][..], &[257u64][..]] {
                    let result = retention_candidates_for_replica_floors(
                        &generations,
                        active,
                        keep,
                        &pins,
                        floors,
                    )
                    .unwrap();
                    let mut expected =
                        retention_candidates_with_pins(&generations, active, keep, &pins).unwrap();
                    expected.retain(|generation| floors.iter().all(|floor| generation < floor));
                    assert_eq!(result, expected);
                }
            }
        }
    }

    #[test]
    fn fails_closed_on_missing_floors_or_stale_pins() {
        let generations = [1, 2, 3, 4];
        assert!(retention_candidates_for_replica_floors(&generations, 4, 1, &[], &[]).is_err());
        assert!(retention_candidates_for_replica_floors(&generations, 4, 1, &[], &[0]).is_err());
        assert!(
            retention_candidates_for_replica_floors(&generations, 4, 1, &[], &[1; 65]).is_err()
        );
        assert!(retention_candidates_for_replica_floors(&generations, 4, 5, &[99], &[1]).is_err());
        assert!(
            retention_candidates_for_replica_floors(&generations, 4, 1, &[2, 2], &[4]).is_err()
        );
    }
}

#[cfg(test)]
mod borrowed_sorted_retention_tests {
    use super::*;

    #[test]
    fn sorted_inventory_avoids_copying_and_unsorted_inventory_still_sorts() {
        let ordered = (1..=16_384u64).collect::<Vec<_>>();
        let unordered = ordered.iter().rev().copied().collect::<Vec<_>>();
        assert!(matches!(
            validated_generations(&ordered, 42).unwrap(),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            validated_generations(&unordered, 42).unwrap(),
            Cow::Owned(_)
        ));

        for active in [1, 1000, 16_384] {
            for keep in [1, 2, 1024, 16_384, usize::MAX] {
                assert_eq!(
                    retention_candidates(&ordered, active, keep).unwrap(),
                    retention_candidates(&unordered, active, keep).unwrap(),
                );
            }
        }
        let pins = [1, 15, 8192, 16384];
        assert_eq!(
            retention_candidates_with_pins(&ordered, 16_384, 1, &pins).unwrap(),
            retention_candidates_with_pins(&unordered, 16_384, 1, &pins).unwrap(),
        );
    }

    #[test]
    fn borrowed_path_still_rejects_invalid_generation_inventories() {
        assert!(validated_generations(&[], 1).is_err());
        assert!(validated_generations(&[0, 1, 2], 2).is_err());
        assert!(validated_generations(&[1, 2, 3], 4).is_err());
        assert!(validated_generations(&[2, 2, 3], 3).is_err());
    }
}

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
    // silently ignored when deleting older generations on another attempt.
    // Avoid a hash table for the common case of a few pinned readers.
    let mut pins =
        (pinned.len() > 4).then(|| std::collections::HashSet::with_capacity(pinned.len()));
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
    let eligible = ordered.len().saturating_sub(keep_latest);
    let mut selected = Vec::new();
    let mut consumed = 0u64;
    for &(id, size) in ordered.iter().take(eligible) {
        if id == active
            || id >= floor
            || pins
                .as_ref()
                .map_or_else(|| pinned.contains(&id), |seen| seen.contains(&id))
        {
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
mod combined_replica_retention_tests {
    use super::*;

    #[test]
    fn oldest_eligible_candidates_skip_pins_and_newer_replica_history() {
        let generations = [
            (8, 10),
            (2, 60),
            (1, 20),
            (7, 2),
            (4, 4),
            (3, 30),
            (6, 5),
            (5, 8),
        ];
        assert_eq!(
            retention_candidates_with_replica_floors_and_budget(
                &generations,
                8,
                2,
                &[2],
                &[5, 7, 6],
                55,
            )
            .unwrap(),
            (vec![1, 3, 4], 54),
        );
        assert_eq!(
            retention_candidates_with_replica_floors_and_budget(
                &generations,
                8,
                2,
                &[2],
                &[1],
                u64::MAX,
            )
            .unwrap(),
            (vec![], 0),
        );
    }

    #[test]
    fn combined_plan_matches_composed_reference_without_wasting_budget() {
        let inventory = (1..=1024u64)
            .rev()
            .map(|id| (id, (id * 31) % 67))
            .collect::<Vec<_>>();
        let ids = inventory.iter().map(|&(id, _)| id).collect::<Vec<_>>();
        let pinned = [4, 20, 509, 800];
        let sizes = inventory
            .iter()
            .copied()
            .collect::<std::collections::HashMap<_, _>>();
        for active in [1, 513, 1024] {
            for keep in [1usize, 5, 128, 1024, usize::MAX] {
                for floors in [&[1u64, 800][..], &[257, 1024][..], &[u64::MAX][..]] {
                    let floor = *floors.iter().min().unwrap();
                    let permitted = retention_candidates_with_pins(&ids, active, keep, &pinned)
                        .unwrap()
                        .into_iter()
                        .filter(|&id| id < floor)
                        .collect::<Vec<_>>();
                    for budget in [0u64, 1, 15, 1024, 10000, u64::MAX] {
                        let mut bytes = 0u64;
                        let mut expected = Vec::new();
                        for &id in &permitted {
                            let size = sizes[&id];
                            if size <= budget - bytes {
                                bytes += size;
                                expected.push(id);
                            }
                        }
                        assert_eq!(
                            retention_candidates_with_replica_floors_and_budget(
                                &inventory, active, keep, &pinned, floors, budget,
                            )
                            .unwrap(),
                            (expected, bytes),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn invalid_replica_floors_or_pins_fail_even_under_zero_budget() {
        let inventory = [(1, 10), (2, 30), (3, 40)];
        for floors in [&[][..], &[0u64][..], &[1u64; 65][..]] {
            assert!(
                retention_candidates_with_replica_floors_and_budget(
                    &inventory,
                    3,
                    1,
                    &[],
                    floors,
                    0,
                )
                .is_err()
            );
        }
        for pins in [&[9u64][..], &[1u64, 1][..]] {
            assert!(
                retention_candidates_with_replica_floors_and_budget(
                    &inventory,
                    3,
                    1,
                    pins,
                    &[2],
                    0,
                )
                .is_err()
            );
        }
        assert!(
            retention_candidates_with_replica_floors_and_budget(
                &[(1, 10), (1, 20)],
                1,
                1,
                &[],
                &[2],
                100,
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod large_inventory_few_pins_tests {
    use super::*;

    #[test]
    fn four_reader_pins_filter_large_inventories_without_a_hash_table() {
        let generations = (1..=16_384u64).collect::<Vec<_>>();
        let pins = [1, 17, 8_192, 16_380];
        let candidates = retention_candidates_with_pins(&generations, 16_384, 2, &pins).unwrap();
        assert_eq!(candidates.len(), 16_382 - pins.len());
        for pin in pins {
            assert!(!candidates.contains(&pin));
        }
        assert_eq!(candidates.first(), Some(&2));
        assert_eq!(candidates.last(), Some(&16_382));
        assert!(
            retention_candidates_with_pins(&generations, 16_384, 2, &[1, 17, 17, 8_192],).is_err()
        );
        assert!(
            retention_candidates_with_pins(&generations, 16_384, 2, &[1, 17, 8_192, 16_385],)
                .is_err()
        );
    }
}

#[cfg(test)]
mod borrowed_budgeted_retention_tests {
    use super::*;

    #[test]
    fn sorted_generation_sizes_borrow_and_unsorted_sizes_sort() {
        let sorted = (1..=2048u64).map(|id| (id, id % 37)).collect::<Vec<_>>();
        let reversed = sorted.iter().copied().rev().collect::<Vec<_>>();
        assert!(matches!(
            ordered_generation_sizes(&sorted),
            Cow::Borrowed(_)
        ));
        assert!(matches!(ordered_generation_sizes(&reversed), Cow::Owned(_)));
        assert_eq!(
            ordered_generation_sizes(&reversed).as_ref(),
            sorted.as_slice()
        );
        for budget in [0, 1, 100, 10_000, u64::MAX] {
            for keep_latest in [1, 2, 512, 2048, usize::MAX] {
                assert_eq!(
                    retention_candidates_with_byte_budget(
                        &sorted,
                        2048,
                        keep_latest,
                        &[1, 9, 37],
                        budget
                    ),
                    retention_candidates_with_byte_budget(
                        &reversed,
                        2048,
                        keep_latest,
                        &[1, 9, 37],
                        budget
                    ),
                );
            }
        }
    }

    #[test]
    fn already_sorted_duplicate_generations_still_fail_validation() {
        assert!(matches!(
            ordered_generation_sizes(&[(1, 1), (1, 2)]),
            Cow::Borrowed(_)
        ));
        assert!(retention_candidates_with_byte_budget(&[(1, 1), (1, 2)], 1, 1, &[], 100).is_err());
    }
}

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
mod min_age_sorted_inventory_tests {
    use super::*;

    #[test]
    fn chronological_and_shuffled_ages_match_exact_retention_policy() {
        let forward = (1..=4_096u64)
            .map(|generation| (generation, generation * 4))
            .collect::<Vec<_>>();
        let reversed = forward.iter().copied().rev().collect::<Vec<_>>();
        let planned = |inventory: &[(u64, u64)]| {
            retention_candidates_with_min_age(
                inventory,
                4_096,
                16,
                &[10, 200],
                &[3_500],
                16_384,
                800,
            )
            .unwrap()
        };
        assert_eq!(planned(&forward), planned(&reversed));
        assert!(planned(&forward).iter().all(|&generation| generation != 10 && generation != 200));
        let mut duplicate = forward.clone();
        duplicate.push((4_096, 16_384));
        assert!(retention_candidates_with_min_age(
            &duplicate, 4_096, 2, &[], &[4_096], 16_384, 0
        ).is_err());
        let mut future = forward;
        future[0].1 = 16_385;
        assert!(retention_candidates_with_min_age(
            &future, 4_096, 2, &[], &[4_096], 16_384, 0
        ).is_err());
    }
}

#[cfg(test)]
mod age_and_replica_floor_retention_tests {
    use super::*;

    #[test]
    fn age_floor_combines_with_active_reader_pins_and_lagging_replicas() {
        let snapshots = [(5, 200), (3, 130), (1, 100), (4, 160), (2, 190)];
        assert_eq!(
            retention_candidates_with_min_age(&snapshots, 5, 1, &[3], &[5], 200, 40).unwrap(),
            vec![1, 4]
        );
        assert_eq!(
            retention_candidates_with_min_age(&snapshots, 5, 1, &[3], &[4], 200, 40).unwrap(),
            vec![1]
        );
        assert_eq!(
            retention_candidates_with_min_age(&snapshots, 5, 1, &[], &[1], 200, 0).unwrap(),
            Vec::<u64>::new()
        );
        assert_eq!(
            retention_candidates_with_min_age(&snapshots, 5, 2, &[], &[5], 200, 40).unwrap(),
            vec![1, 3]
        );
    }

    #[test]
    fn older_minimum_age_never_expands_retention_candidates() {
        let generations = (1..=256u64).map(|id| (id, id * 3)).collect::<Vec<_>>();
        let mut previous =
            retention_candidates_with_min_age(&generations, 256, 1, &[], &[256], 768, 0).unwrap();
        for age in [1, 3, 24, 96, 384, 768, u64::MAX] {
            let candidate =
                retention_candidates_with_min_age(&generations, 256, 1, &[], &[256], 768, age)
                    .unwrap();
            assert!(
                candidate
                    .iter()
                    .all(|generation| previous.contains(generation))
            );
            previous = candidate;
        }
    }

    #[test]
    fn rejects_future_timestamps_stale_pins_and_missing_replica_floors() {
        let snapshots = [(1, 100), (2, 101)];
        assert!(retention_candidates_with_min_age(&snapshots, 2, 1, &[], &[2], 100, 0).is_err());
        assert!(retention_candidates_with_min_age(&snapshots, 2, 1, &[3], &[2], 101, 0).is_err());
        assert!(retention_candidates_with_min_age(&snapshots, 2, 1, &[], &[], 101, 0).is_err());
        assert!(
            retention_candidates_with_min_age(&[(1, 0), (1, 0)], 1, 1, &[], &[1], 0, 0).is_err()
        );
    }
}

#[cfg(test)]
mod sorted_replica_aware_inventory_tests {
    use super::*;

    #[test]
    fn borrowed_and_reversed_inventories_produce_identical_retention_plans() {
        let sorted = (1..=10_000u64)
            .map(|generation| (generation, generation % 7 + 1))
            .collect::<Vec<_>>();
        assert!(matches!(
            ordered_generation_sizes(&sorted),
            Cow::Borrowed(_)
        ));
        let mut reversed = sorted.clone();
        reversed.reverse();
        assert!(matches!(ordered_generation_sizes(&reversed), Cow::Owned(_)));

        for pins in [
            &[][..],
            &[77, 500, 9000][..],
            &[77, 500, 900, 2000, 9000][..],
        ] {
            let expected = retention_candidates_with_replica_floors_and_budget(
                &sorted,
                10_000,
                3,
                pins,
                &[9_000, 9_700],
                3_000,
            )
            .unwrap();
            let actual = retention_candidates_with_replica_floors_and_budget(
                &reversed,
                10_000,
                3,
                pins,
                &[9_000, 9_700],
                3_000,
            )
            .unwrap();
            assert_eq!(actual, expected);
            assert!(actual.0.iter().all(|&generation| generation < 9_000));
            assert!(actual.0.iter().all(|generation| !pins.contains(generation)));
            assert!(actual.1 <= 3_000);
        }

        assert!(
            retention_candidates_with_replica_floors_and_budget(
                &sorted,
                10_000,
                3,
                &[77, 77],
                &[9000],
                0
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod ordered_replica_pin_merge_tests {
    use super::*;

    #[test]
    fn sorted_and_shuffled_pins_produce_the_same_retention_plans() {
        let generations = (1..=4096u64).rev().collect::<Vec<_>>();
        let sizes = generations
            .iter()
            .map(|&id| (id, (id * 23) % 991))
            .collect::<Vec<_>>();
        for pins in [
            vec![2, 3, 11, 1024, 2048],
            vec![1, 128, 256, 512, 1024, 2048, 4095],
            (1..=4096).filter(|i| i % 5 == 0).collect::<Vec<_>>(),
        ] {
            let mut reversed = pins.clone();
            reversed.reverse();
            for keep in [1usize, 128, 4096] {
                assert_eq!(
                    retention_candidates_with_pins(&generations, 4096, keep, &pins).unwrap(),
                    retention_candidates_with_pins(&generations, 4096, keep, &reversed).unwrap(),
                );
                for budget in [0u64, 200, 10_000, u64::MAX] {
                    assert_eq!(
                        retention_candidates_with_byte_budget(&sizes, 4096, keep, &pins, budget,)
                            .unwrap(),
                        retention_candidates_with_byte_budget(
                            &sizes, 4096, keep, &reversed, budget,
                        )
                        .unwrap(),
                    );
                }
            }
        }
    }

    #[test]
    fn sorted_pin_fast_path_never_accepts_unknown_or_duplicate_ids() {
        let generations = (1..=32u64).collect::<Vec<_>>();
        let sizes = generations.iter().map(|&id| (id, 1u64)).collect::<Vec<_>>();
        for pins in [vec![1, 2, 2, 3, 4, 5], vec![1, 2, 3, 4, 100]] {
            assert!(retention_candidates_with_pins(&generations, 32, 2, &pins,).is_err());
            assert!(retention_candidates_with_byte_budget(&sizes, 32, 2, &pins, 100,).is_err());
        }
    }
}
