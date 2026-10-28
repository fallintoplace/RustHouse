//! Conservative generation-retention planning; no files are deleted.
/// List old generations eligible for review by a caller with exclusive ownership.
/// Always preserve the active generation and the newest keep_latest generations.
pub fn retention_candidates(
    generations: &[u64],
    active: u64,
    keep_latest: usize,
) -> Result<Vec<u64>, &'static str> {
    if generations.is_empty()
        || generations.len() > 16_384
        || keep_latest == 0
        || active == 0
    {
        return Err("invalid generation retention configuration");
    }
    let mut sorted = generations.to_vec();
    sorted.sort_unstable();
    if sorted[0] == 0
        || sorted.windows(2).any(|pair| pair[0] == pair[1])
        || sorted.binary_search(&active).is_err()
    {
        return Err("duplicate, missing, or invalid generation");
    }
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
        assert_eq!(
            retention_candidates(&[4, 1, 3, 2], 2, 2).unwrap(),
            vec![1]
        );
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
        assert_eq!(retention_candidates(&[5, 3, 1, 4, 2], 1, 2).unwrap(), vec![2, 3]);
        assert_eq!(retention_candidates(&[5, 3, 1, 4, 2], 5, 2).unwrap(), vec![1, 2, 3]);
        assert!(retention_candidates(&[1, 2, 3], 1, usize::MAX).unwrap().is_empty());
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