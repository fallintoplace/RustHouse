//! Conservative generation-retention planning; no files are deleted.

const MAX_RETENTION_GENERATIONS: usize = 16_384;
fn validated_generations(generations: &[u64], active: u64) -> Result<Vec<u64>, &'static str> {
    let mut sorted_generations = generations.to_vec();
    sorted_generations.sort_unstable();
    if sorted_generations.first().is_none_or(|generation| *generation == 0)
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

fn planned_retention(
    generations: &[u64],
    active: u64,
    keep_latest: usize,
) -> Result<(Vec<u64>, Vec<u64>), &'static str> {
    if generations.is_empty() || generations.len() > MAX_RETENTION_GENERATIONS || keep_latest == 0 || active == 0 {
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
        return Ok(candidates.into_iter().filter(|generation| generation != pin).collect());
    }
    if pinned.len() <= 4 && candidates.len() <= 128 {
        for (index, &generation) in pinned.iter().enumerate() {
            if pinned[..index].contains(&generation) || !known_generation(&known, generation) {
                return Err("duplicate or unknown pinned generation");
            }
        }
        return Ok(candidates.into_iter()
            .filter(|generation| !pinned.contains(generation))
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
    let mut ordered = generations.to_vec();
    ordered.sort_unstable_by_key(|&(generation, _)| generation);
    if ordered[0].0 == 0
        || ordered.windows(2).any(|pair| pair[0].0 == pair[1].0)
        || ordered.binary_search_by_key(&active, |&(id, _)| id).is_err()
    {
        return Err("duplicate, missing, or invalid generation");
    }
    // Validate every pin even if no generation can be reclaimed.
    let mut pin_set = (pinned.len() > 4)
        .then(|| std::collections::HashSet::with_capacity(pinned.len()));
    for (index, &generation) in pinned.iter().enumerate() {
        let duplicate = if let Some(seen) = pin_set.as_mut() {
            !seen.insert(generation)
        } else {
            pinned[..index].contains(&generation)
        };
        if duplicate || ordered.binary_search_by_key(&generation, |&(id, _)| id).is_err() {
            return Err("duplicate or unknown pinned generation");
        }
    }
    let candidates = ordered.len().saturating_sub(keep_latest);
    let mut selected = Vec::new();
    let mut total = 0u64;
    for &(generation, bytes) in ordered.iter().take(candidates) {
        if generation == active
            || (pin_set.as_ref().map_or_else(
                || pinned.contains(&generation),
                |pins| pins.contains(&generation),
            ))
        {
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
        let generations = (1..=4096u64).rev()
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
                let bytes = sizes.binary_search_by_key(&id, |&(generation, _)| generation)
                    .map(|index| sizes[index].1).unwrap();
                if bytes <= budget - used {
                    used += bytes;
                    expected.push(id);
                }
            }
            assert_eq!(
                retention_candidates_with_byte_budget(
                    &generations, 4096, 100, &pinned, budget,
                ).unwrap(),
                (expected, used),
            );
        }
    }

    #[test]
    fn rejects_invalid_pins_even_when_every_generation_is_kept() {
        assert!(retention_candidates_with_byte_budget(
            &[(1, 4), (2, 8)], 2, 2, &[1, 1], 100,
        ).is_err());
        assert!(retention_candidates_with_byte_budget(
            &[(1, 4), (2, 8)], 2, 2, &[3], 100,
        ).is_err());
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
            (vec![1], 7)
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
                &generations, 16_384, generations.len(), &[], u64::MAX,
            ).unwrap(),
            (Vec::<u64>::new(), 0),
        );
        assert!(retention_candidates_with_byte_budget(
            &[(1, 4), (1, 8)], 1, 2, &[], u64::MAX,
        ).is_err());
    }
}

#[cfg(test)]
mod small_pin_retention_tests {
    use super::*;

    #[test]
    fn low_cardinality_pins_preserve_large_path_semantics() {
        let generations = (1..=130u64).collect::<Vec<_>>();
        for pins in [vec![], vec![1], vec![1, 3], vec![1, 3, 5, 7], vec![1, 3, 5, 7, 9]] {
            let small = retention_candidates_with_pins(&generations, 130, 2, &pins).unwrap();
            assert_eq!(small, (1..=128u64).filter(|id| !pins.contains(id)).collect::<Vec<_>>());
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
        assert_eq!(retention_candidates_with_byte_budget(&sizes, 4, 3, &[], 24).unwrap(), (vec![], 0));
        assert_eq!(retention_candidates_with_byte_budget(&sizes, 4, 3, &[], 25).unwrap(), (vec![1], 25));
        assert_eq!(retention_candidates_with_byte_budget(&sizes, 4, 3, &[], u64::MAX).unwrap(), (vec![1], 25));
        assert_eq!(retention_candidates_with_byte_budget(&sizes, 4, 3, &[1], u64::MAX).unwrap(), (vec![], 0));
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
    Ok(retention_candidates_with_pins(generations, active, keep_latest, pinned)?
        .into_iter()
        .filter(|&generation| generation < oldest_needed)
        .collect())
}

#[cfg(test)]
mod multiple_replica_floor_tests {
    use super::*;

    #[test]
    fn slowest_replica_and_reader_pins_are_jointly_protected() {
        let generations = [1, 2, 3, 4, 5, 6, 7];
        let selected = retention_candidates_for_replica_floors(
            &generations, 7, 2, &[2], &[5, 3, 6],
        ).unwrap();
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
                        &generations, active, keep, &pins, floors,
                    ).unwrap();
                    let mut expected = retention_candidates_with_pins(
                        &generations, active, keep, &pins,
                    ).unwrap();
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
        assert!(retention_candidates_for_replica_floors(&generations, 4, 1, &[], &[1; 65]).is_err());
        assert!(retention_candidates_for_replica_floors(&generations, 4, 5, &[99], &[1]).is_err());
        assert!(retention_candidates_for_replica_floors(&generations, 4, 1, &[2, 2], &[4]).is_err());
    }
}
