//! Conservative generation-retention planning; no files are deleted.
use std::collections::BTreeSet;

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
    let unique: BTreeSet<u64> = generations.iter().copied().collect();
    if unique.len() != generations.len() || unique.contains(&0) || !unique.contains(&active) {
        return Err("duplicate, missing, or invalid generation");
    }
    let preserved: BTreeSet<u64> = unique.iter().rev().take(keep_latest).copied().collect();
    Ok(unique
        .into_iter()
        .filter(|generation| *generation != active && !preserved.contains(generation))
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
}