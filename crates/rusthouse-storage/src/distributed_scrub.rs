//! Read-only, bounded verification of an immutable replicated snapshot.
use crate::{
    PartError, query_parts, resolve_active_snapshot, verify_part_file_hashes, verify_wide_part,
};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScrubReport {
    pub parts: usize,
    pub checked_files: usize,
    pub checked_bytes: u64,
    pub blocks: u64,
}

impl ScrubReport {
    fn record_part(&mut self, files: usize, bytes: u64, blocks: u64) -> Result<(), PartError> {
        self.checked_files = self
            .checked_files
            .checked_add(files)
            .ok_or_else(|| PartError::InvalidScan("scrub file count overflow".into()))?;
        self.checked_bytes = self
            .checked_bytes
            .checked_add(bytes)
            .ok_or_else(|| PartError::InvalidScan("scrub byte count overflow".into()))?;
        self.blocks = self
            .blocks
            .checked_add(blocks)
            .ok_or_else(|| PartError::InvalidScan("scrub block count overflow".into()))?;
        self.parts += 1;
        Ok(())
    }
}

/// Check the physical structure and stored-byte hashes of every active part.
/// Stop writers first. This does not repair or mutate replicas.
pub fn scrub_snapshot(root: &Path, max_parts: usize) -> Result<ScrubReport, PartError> {
    scrub_snapshot_with_byte_limit(root, max_parts, u64::MAX)
}

/// Reject oversized snapshots before starting expensive block/hash verification.
pub fn scrub_snapshot_with_byte_limit(
    root: &Path,
    max_parts: usize,
    max_bytes: u64,
) -> Result<ScrubReport, PartError> {
    let active = resolve_active_snapshot(root)?;
    let parts = query_parts(&active)?;
    if parts.len() > max_parts || max_parts > 16_384 {
        return Err(PartError::InvalidScan(
            "replica scrub exceeds part limit".into(),
        ));
    }
    if max_bytes != u64::MAX {
        crate::distributed_inventory::inventory_snapshot(root, 1_048_576, max_bytes)?;
    }
    const PARALLEL_THRESHOLD: usize = 16;
    const MAX_SCRUB_WORKERS: usize = 4;
    let workers = std::thread::available_parallelism()
        .map_or(1, |parallelism| parallelism.get())
        .min(MAX_SCRUB_WORKERS);
    let mut report = ScrubReport::default();
    if parts.len() < PARALLEL_THRESHOLD || workers == 1 {
        for part in &parts {
            let checked = scrub_part(part)?;
            report.record_part(checked.checked_files, checked.checked_bytes, checked.blocks)?;
        }
        return Ok(report);
    }

    // Scrub independent frozen parts concurrently. Dynamic work claiming
    // balances uneven part sizes, with deterministic first-error reporting.
    use std::sync::atomic::{AtomicUsize, Ordering};
    let cursor = AtomicUsize::new(0);
    let mut slots: Vec<Option<Result<ScrubReport, PartError>>> =
        (0..parts.len()).map(|_| None).collect();
    std::thread::scope(|scope| -> Result<(), PartError> {
        let handles = (0..workers)
            .map(|_| {
                let cursor = &cursor;
                let parts = &parts;
                scope.spawn(move || {
                    let mut completed = Vec::new();
                    loop {
                        let index = cursor.fetch_add(1, Ordering::Relaxed);
                        if index >= parts.len() {
                            break;
                        }
                        completed.push((index, scrub_part(&parts[index])));
                    }
                    completed
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            let completed = handle.join().map_err(|_| {
                PartError::InvalidScan("snapshot scrub worker panicked".into())
            })?;
            for (index, result) in completed {
                slots[index] = Some(result);
            }
        }
        Ok(())
    })?;
    for slot in slots {
        let checked = slot.expect("each part was claimed exactly once")?;
        report.record_part(checked.checked_files, checked.checked_bytes, checked.blocks)?;
    }
    Ok(report)
}

/// Verify one frozen wide part without scanning unrelated snapshot parts.
/// The caller must stop writers and provide a trusted local part path.
pub fn scrub_part(part: &Path) -> Result<ScrubReport, PartError> {
    let physical = verify_wide_part(part)?;
    let hashes = verify_part_file_hashes(part)?;
    let mut report = ScrubReport::default();
    report.record_part(hashes.files, hashes.total_bytes, physical.blocks)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Column, write_wide_part};
    use std::fs;

    #[test]
    fn scrubs_large_multi_part_snapshot_without_losing_report_counts() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-parallel-scrub-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        for index in 0..20 {
            let part = root.join(format!("all_{index}_{index}_0"));
            write_wide_part(
                &part,
                &[Column { name: "id".into(), data_type: "UInt64".into() }],
                &[vec![Some(index.to_string())]],
            ).unwrap();
        }
        let report = scrub_snapshot(&root, 20).unwrap();
        assert_eq!(report.parts, 20);
        assert!(report.checked_files >= 20);
        assert!(report.checked_bytes > 0);
        fs::write(root.join("all_19_19_0/count.txt"), "999999\n").unwrap();
        assert!(scrub_snapshot(&root, 20).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verifies_newly_written_frozen_parts() {
        let root = std::env::temp_dir().join(format!("rusthouse-scrub-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        write_wide_part(
            &root.join("all_0_0_0"),
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("4".into())]],
        )
        .unwrap();
        let result = scrub_snapshot(&root, 1).unwrap();
        assert_eq!(result.parts, 1);
        assert!(result.checked_files > 0);
        assert!(scrub_snapshot(&root, 0).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn refuses_tampered_snapshot_files() {
        let root =
            std::env::temp_dir().join(format!("rusthouse-scrub-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let part = root.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("4".into())]],
        )
        .unwrap();
        fs::write(part.join("count.txt"), "999\n").unwrap();
        assert!(scrub_snapshot(&root, 1).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod targeted_scrub_tests {
    use super::*;
    use crate::{Column, write_wide_part};
    use std::fs;

    #[test]
    fn verifies_one_part_and_rejects_corruption() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-targeted-scrub-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let part = root.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column { name: "id".into(), data_type: "UInt64".into() }],
            &[vec![Some("8".into())]],
        ).unwrap();
        let report = scrub_part(&part).unwrap();
        assert_eq!(report.parts, 1);
        assert!(report.checked_files > 0);
        fs::write(part.join("count.txt"), "999\n").unwrap();
        assert!(scrub_part(&part).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod byte_budget_tests {
    use super::*;
    use crate::{Column, write_wide_part};
    use std::fs;

    #[test]
    fn rejects_oversized_scrubs_without_touching_snapshot_files() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-scrub-budget-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write_wide_part(
            &root.join("all_0_0_0"),
            &[Column { name: "id".into(), data_type: "UInt64".into() }],
            &[vec![Some("4".into())]],
        )
        .unwrap();
        assert!(scrub_snapshot_with_byte_limit(&root, 1, 1).is_err());
        let report = scrub_snapshot_with_byte_limit(&root, 1, 1_000_000).unwrap();
        assert_eq!(report.parts, 1);
        assert!(report.checked_bytes > 1);
        fs::remove_dir_all(root).unwrap();
    }
}
