//! Conservative generation-retention planning; no files are deleted.
fn validated_generations(generations: &[u64], active: u64) -> Result<Vec<u64>, &'static str> {
    let mut sorted = generations.to_vec();
    sorted.sort_unstable();
    if sorted[0] == 0
        || sorted.windows(2).any(|pair| pair[0] == pair[1])
        || sorted.binary_search(&active).is_err()
    {
        return Err("duplicate, missing, or invalid generation");
    }
    Ok(sorted)
}

/// List old generations eligible for review by a caller with exclusive ownership.
/// Always preserve the active generation and the newest keep_latest generations.
pub fn retention_candidates(
    generations: &[u64],
    active: u64,
    keep_latest: usize,
) -> Result<Vec<u64>, &'static str> {
    if generations.is_empty() || generations.len() > 16_384 || keep_latest == 0 || active == 0 {
        return Err("invalid generation retention configuration");
    }
    let sorted = validated_generations(generations, active)?;
    let candidate_count = sorted.len().saturating_sub(keep_latest);
    Ok(sorted
        .into_iter()
        .take(candidate_count)
        .filter(|&generation| generation != active)
        .collect())
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
    if pinned.len() > 16_384 {
        return Err("too many pinned generations");
    }
    let candidates = retention_candidates(generations, active, keep_latest)?;
    let known = validated_generations(generations, active)?;
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
    fn rejects_stale_and_duplicate_pins() {
        assert!(retention_candidates_with_pins(&[1, 2, 3], 3, 1, &[4]).is_err());
        assert!(retention_candidates_with_pins(&[1, 2, 3], 3, 1, &[1, 1]).is_err());
        assert!(retention_candidates_with_pins(&[1, 1], 1, 1, &[]).is_err());
    }
}
