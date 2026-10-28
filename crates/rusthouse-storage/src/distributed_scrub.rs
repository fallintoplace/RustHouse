//! Read-only, bounded verification of an immutable replicated snapshot.
use crate::{
    PartError, query_parts, resolve_active_snapshot, verify_part_file_hashes, verify_wide_part,
};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrubReport {
    pub parts: usize,
    pub checked_files: usize,
    pub checked_bytes: u64,
    pub blocks: u64,
}

/// Check the physical structure and stored-byte hashes of every active part.
/// Stop writers first. This does not repair or mutate replicas.
pub fn scrub_snapshot(root: &Path, max_parts: usize) -> Result<ScrubReport, PartError> {
    let active = resolve_active_snapshot(root)?;
    let parts = query_parts(&active)?;
    if parts.len() > max_parts || max_parts > 16_384 {
        return Err(PartError::InvalidScan("replica scrub exceeds part limit".into()));
    }
    let mut report = ScrubReport {
        parts: 0,
        checked_files: 0,
        checked_bytes: 0,
        blocks: 0,
    };
    for part in parts {
        let physical = verify_wide_part(&part)?;
        let hashes = verify_part_file_hashes(&part)?;
        report.parts += 1;
        report.checked_files = report
            .checked_files
            .checked_add(hashes.files)
            .ok_or_else(|| PartError::InvalidScan("scrub file count overflow".into()))?;
        report.checked_bytes = report
            .checked_bytes
            .checked_add(hashes.total_bytes)
            .ok_or_else(|| PartError::InvalidScan("scrub byte count overflow".into()))?;
        report.blocks = report
            .blocks
            .checked_add(physical.blocks)
            .ok_or_else(|| PartError::InvalidScan("scrub block count overflow".into()))?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Column, write_wide_part};
    use std::fs;

    #[test]
    fn verifies_newly_written_frozen_parts() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-scrub-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        write_wide_part(
            &root.join("all_0_0_0"),
            &[Column { name: "id".into(), data_type: "UInt64".into() }],
            &[vec![Some("4".into())]],
        )
        .unwrap();
        let result = scrub_snapshot(&root, 1).unwrap();
        assert_eq!(result.parts, 1);
        assert!(result.checked_files > 0);
        assert!(scrub_snapshot(&root, 0).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}