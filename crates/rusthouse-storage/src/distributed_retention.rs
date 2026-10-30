//! Conservative generation-retention planning; no files are deleted.

const MAX_RETENTION_GENERATIONS: usize = 16_384;
fn validated_generations(generations: &[u64], active: u64) -> Result<Vec<u64>, &'static str> {
    let mut sorted_generations = generations.to_vec();
    sorted_generations.sort_unstable();
    if sorted_generations[0] == 0
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
    if generations.is_empty() || generations.len() > 16_384 {
        return Err("invalid generation retention inventory");
    }
    let ids = generations.iter().map(|&(generation, _)| generation).collect::<Vec<_>>();
    let eligible = retention_candidates_with_pins(&ids, active, keep_latest, pinned)?;
    if eligible.is_empty() {
        return Ok((Vec::new(), 0));
    }
    if let [generation] = eligible.as_slice() {
        // A single eligible generation only needs a lookup, not a full size index.
        let bytes = generations.iter()
            .find(|(id, _)| id == generation)
            .expect("validated generation exists in retention inventory")
            .1;
        return if bytes <= byte_budget {
            Ok((vec![*generation], bytes))
        } else {
            Ok((Vec::new(), 0))
        };
    }
    let sizes = generations.iter().copied().collect::<std::collections::HashMap<_, _>>();
    let mut selected = Vec::new();
    let mut total = 0u64;
    for generation in eligible {
        let bytes = sizes[&generation];
        if bytes <= byte_budget - total {
            total += bytes;
            selected.push(generation);
        }
    }
    Ok((selected, total))
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
