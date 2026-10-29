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
    let mut pins = std::collections::HashSet::with_capacity(pinned.len());
    for &generation in pinned {
        if !pins.insert(generation) || known.binary_search(&generation).is_err() {
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
