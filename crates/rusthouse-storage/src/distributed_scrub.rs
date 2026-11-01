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
        // Validate every counter before mutating the report. Late overflow
        // must not leave the file/byte totals updated without their part.
        let checked_files = self
            .checked_files
            .checked_add(files)
            .ok_or_else(|| PartError::InvalidScan("scrub file count overflow".into()))?;
        let checked_bytes = self
            .checked_bytes
            .checked_add(bytes)
            .ok_or_else(|| PartError::InvalidScan("scrub byte count overflow".into()))?;
        let blocks = self
            .blocks
            .checked_add(blocks)
            .ok_or_else(|| PartError::InvalidScan("scrub block count overflow".into()))?;
        let parts = self
            .parts
            .checked_add(1)
            .ok_or_else(|| PartError::InvalidScan("scrub part count overflow".into()))?;
        self.checked_files = checked_files;
        self.checked_bytes = checked_bytes;
        self.blocks = blocks;
        self.parts = parts;
        Ok(())
    }
}

fn require_unchanged_snapshot(root: &Path, active: &Path) -> Result<(), PartError> {
    if resolve_active_snapshot(root)? != active {
        return Err(PartError::InvalidScan(
            "active snapshot changed during replica scrub".into(),
        ));
    }
    Ok(())
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
        // The active generation was resolved above. Avoid resolving it a
        // second time while preflighting the byte budget.
        crate::distributed_inventory::inventory_active_snapshot(&active, 1_048_576, max_bytes)?;
    }
    const PARALLEL_THRESHOLD: usize = 16;
    let mut report = ScrubReport::default();
    if parts.len() < PARALLEL_THRESHOLD {
        for part in &parts {
            let checked = scrub_part(part)?;
            report.record_part(checked.checked_files, checked.checked_bytes, checked.blocks)?;
        }
        require_unchanged_snapshot(root, &active)?;
        return Ok(report);
    }
    const MAX_SCRUB_WORKERS: usize = 4;
    let workers = std::thread::available_parallelism()
        .map_or(1, |parallelism| parallelism.get())
        .min(MAX_SCRUB_WORKERS);
    if workers == 1 {
        for part in &parts {
            let checked = scrub_part(part)?;
            report.record_part(checked.checked_files, checked.checked_bytes, checked.blocks)?;
        }
        require_unchanged_snapshot(root, &active)?;
        return Ok(report);
    }

    // Scrub independent frozen parts concurrently. Dynamic work claiming
    // balances uneven part sizes, with deterministic first-error reporting.
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let cursor = AtomicUsize::new(0);
    // Keep the lowest failing index. Work already claimed before a failure
    // finishes normally; unclaimed later parts do not need to be scanned.
    let first_failure = AtomicUsize::new(usize::MAX);
    // The atomic index gives each part one owner. Keep only one result slot
    // per part instead of also retaining a full vector inside every worker.
    let slots: Vec<OnceLock<Result<ScrubReport, PartError>>> =
        (0..parts.len()).map(|_| OnceLock::new()).collect();
    std::thread::scope(|scope| -> Result<(), PartError> {
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let cursor = &cursor;
            let parts = &parts;
            let slots = &slots;
            let first_failure = &first_failure;
            let handle = std::thread::Builder::new()
                .spawn_scoped(scope, move || {
                    loop {
                        let index = cursor.fetch_add(1, Ordering::Relaxed);
                        if index >= parts.len() || index > first_failure.load(Ordering::Acquire) {
                            break;
                        }
                        let result = scrub_part(&parts[index]);
                        if result.is_err() {
                            first_failure.fetch_min(index, Ordering::AcqRel);
                        }
                        slots[index]
                            .set(result)
                            .unwrap_or_else(|_| unreachable!("scrub index claimed twice"));
                    }
                })
                .map_err(|error| {
                    PartError::InvalidScan(format!("cannot start scrub worker: {error}"))
                })?;
            handles.push(handle);
        }
        for handle in handles {
            handle
                .join()
                .map_err(|_| PartError::InvalidScan("snapshot scrub worker panicked".into()))?;
        }
        Ok(())
    })?;
    // Every index below the first failure was already claimed because the
    // cursor grows monotonically. Once all workers have joined, report that
    // earliest failure and ignore intentionally unclaimed later indexes.
    let first_failure = first_failure.load(Ordering::Acquire);
    for (index, slot) in slots.into_iter().enumerate() {
        if index > first_failure {
            break;
        }
        let checked = slot
            .into_inner()
            .ok_or_else(|| PartError::InvalidScan("missing snapshot scrub result".into()))??;
        report.record_part(checked.checked_files, checked.checked_bytes, checked.blocks)?;
    }
    require_unchanged_snapshot(root, &active)?;
    Ok(report)
}

/// Verify one frozen wide part without scanning unrelated snapshot parts.
/// The caller must stop writers and provide a trusted local part path.
pub fn scrub_part(part: &Path) -> Result<ScrubReport, PartError> {
    // Check immutable stored bytes before decoding compressed blocks or
    // interpreting their metadata. Corrupt replicas fail fast and do not
    // feed known-bad input into the more expensive structural verifier.
    let hashes = verify_part_file_hashes(part)?;
    let physical = verify_wide_part(part)?;
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
        let root =
            std::env::temp_dir().join(format!("rusthouse-parallel-scrub-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        for index in 0..20 {
            let part = root.join(format!("all_{index}_{index}_0"));
            write_wide_part(
                &part,
                &[Column {
                    name: "id".into(),
                    data_type: "UInt64".into(),
                }],
                &[vec![Some(index.to_string())]],
            )
            .unwrap();
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
        let root =
            std::env::temp_dir().join(format!("rusthouse-targeted-scrub-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let part = root.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("8".into())]],
        )
        .unwrap();
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
        let root =
            std::env::temp_dir().join(format!("rusthouse-scrub-budget-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        write_wide_part(
            &root.join("all_0_0_0"),
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
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

#[cfg(test)]
mod pinned_scrub_preflight_tests {
    use super::*;
    use crate::{Column, write_wide_part};
    use std::fs;

    #[test]
    fn scrub_budget_is_applied_to_selected_active_generation() {
        let root =
            std::env::temp_dir().join(format!("rusthouse-scrub-generation-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let versions = root.join(".rusthouse-versions");
        let first = versions.join("g1");
        let second = versions.join("g2");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        write_wide_part(
            &first.join("all_0_0_0"),
            &[Column {
                name: "value".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("1".into())]],
        )
        .unwrap();
        write_wide_part(
            &second.join("all_0_0_0"),
            &[Column {
                name: "value".into(),
                data_type: "String".into(),
            }],
            &[vec![Some("a".repeat(512))]],
        )
        .unwrap();
        fs::write(root.join(".rusthouse-active"), "g1\n").unwrap();
        let first_bytes = crate::distributed_inventory::inventory_snapshot(&first, 1024, u64::MAX)
            .unwrap()
            .iter()
            .map(|e| e.bytes)
            .sum::<u64>();
        assert!(scrub_snapshot_with_byte_limit(&root, 1, first_bytes).is_ok());
        fs::write(root.join(".rusthouse-active"), "g2\n").unwrap();
        assert!(scrub_snapshot_with_byte_limit(&root, 1, first_bytes).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod scrub_generation_stability_tests {
    use super::*;
    use std::fs;

    #[test]
    fn active_generation_switch_is_not_a_successful_scrub() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-scrub-generation-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&root);
        let first = root.join(".rusthouse-versions/g1");
        let second = root.join(".rusthouse-versions/g2");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let pointer = root.join(".rusthouse-active");
        fs::write(&pointer, "g1\n").unwrap();
        assert!(require_unchanged_snapshot(&root, &first).is_ok());
        fs::write(&pointer, "g2\n").unwrap();
        assert!(require_unchanged_snapshot(&root, &first).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod fail_fast_scrub_tests {
    use super::*;
    use crate::{Column, write_wide_part};
    use std::fs;

    #[test]
    fn returns_earliest_corrupt_part_with_parallel_workers() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-fail-fast-scrub-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for index in 0..32 {
            write_wide_part(
                &root.join(format!("all_{index}_{index}_0")),
                &[Column {
                    name: "id".into(),
                    data_type: "UInt64".into(),
                }],
                &[vec![Some(index.to_string())]],
            )
            .unwrap();
        }
        assert_eq!(scrub_snapshot(&root, 32).unwrap().parts, 32);
        fs::write(root.join("all_1_1_0/count.txt"), "999\n").unwrap();
        fs::write(root.join("all_31_31_0/count.txt"), "888\n").unwrap();
        let error = scrub_snapshot(&root, 32).unwrap_err();
        // A corrupted part must never produce a successful full scrub.
        assert!(!format!("{error:?}").is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod hash_first_scrub_tests {
    use super::*;
    use crate::{Column, write_wide_part};
    use std::fs;

    #[test]
    fn corrupted_stream_fails_hash_validation_before_decoding() {
        let root =
            std::env::temp_dir().join(format!("rusthouse-scrub-hash-first-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let part = root.join("all_0_0_0");
        write_wide_part(
            &part,
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("7".into())], vec![Some("8".into())]],
        )
        .unwrap();
        assert!(scrub_part(&part).is_ok());
        let stream = part.join("id.bin");
        let mut content = fs::read(&stream).unwrap();
        assert!(!content.is_empty());
        content[0] ^= 0xff;
        fs::write(&stream, content).unwrap();
        assert!(matches!(
            scrub_part(&part),
            Err(PartError::InvalidChecksums { .. }),
        ));
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod resolved_scrub_preflight_tests {
    use super::*;
    use crate::{Column, write_wide_part};
    use std::fs;

    #[test]
    fn bounded_scrub_preflight_uses_the_selected_generation() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-scrub-resolved-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&root);
        let old = root.join(".rusthouse-versions/g1");
        let newer = root.join(".rusthouse-versions/g2");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&newer).unwrap();
        write_wide_part(
            &old.join("all_0_0_0"),
            &[Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("1".into())]],
        )
        .unwrap();
        write_wide_part(
            &newer.join("all_0_0_0"),
            &[Column {
                name: "text".into(),
                data_type: "String".into(),
            }],
            &[vec![Some("x".repeat(8192))]],
        )
        .unwrap();
        fs::write(root.join(".rusthouse-active"), "g1\n").unwrap();
        let old_bytes =
            crate::distributed_inventory::inventory_active_snapshot(&old, 128, u64::MAX)
                .unwrap()
                .iter()
                .map(|file| file.bytes)
                .sum();
        assert!(scrub_snapshot_with_byte_limit(&root, 1, old_bytes).is_ok());
        fs::write(root.join(".rusthouse-active"), "g2\n").unwrap();
        assert!(scrub_snapshot_with_byte_limit(&root, 1, old_bytes).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

/// Resolve a continuation cursor without rescanning up to 16,384 parts.
/// The caller must sort the paths by file name before this binary search.
fn scrub_cursor_index(parts: &[std::path::PathBuf], after: &str) -> Result<usize, PartError> {
    if after.is_empty() || after.contains('/') || after.contains('\\') {
        return Err(PartError::InvalidScan(
            "invalid snapshot scrub cursor".into(),
        ));
    }
    let name = Some(std::ffi::OsStr::new(after));
    parts
        .binary_search_by(|part| part.file_name().cmp(&name))
        .map(|index| index + 1)
        .map_err(|_| {
            PartError::InvalidScan(
                "snapshot scrub cursor was not found in active generation".into(),
            )
        })
}

/// A completed page of read-only immutable-part verification.
///
/// Keep generation unchanged when passing its cursor to the next page.
/// An untrusted caller must not interpret the cursor as proof of consistency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagedScrubReport {
    pub generation: std::path::PathBuf,
    pub verified: ScrubReport,
    /// Exclusive cursor; None means every part in the snapshot was checked.
    pub next_after_part: Option<String>,
}

/// Verify at most 256 frozen active parts per invocation, with a strict
/// lexicographic cursor and an optional pinned-generation requirement.
///
/// Stop writers before scrubbing. A page is not a transaction, and no
/// generation or cursor may authorize repair or data deletion.
pub fn scrub_snapshot_page(
    root: &Path,
    pinned_generation: Option<&Path>,
    after_part: Option<&str>,
    max_page_parts: usize,
) -> Result<PagedScrubReport, PartError> {
    if max_page_parts == 0 || max_page_parts > 256 {
        return Err(PartError::InvalidScan(
            "invalid snapshot scrub page size".into(),
        ));
    }
    let active = resolve_active_snapshot(root)?;
    if pinned_generation.is_some_and(|expected| expected != active) {
        return Err(PartError::InvalidScan(
            "snapshot scrub continuation changed active generation".into(),
        ));
    }
    let mut parts = query_parts(&active)?;
    if parts.len() > 16_384 {
        return Err(PartError::InvalidScan(
            "replica scrub exceeds part limit".into(),
        ));
    }
    parts.sort_unstable_by(|left, right| left.file_name().cmp(&right.file_name()));
    let start = after_part.map_or(Ok(0), |after| scrub_cursor_index(&parts, after))?;
    let end = start.saturating_add(max_page_parts).min(parts.len());
    let mut report = ScrubReport::default();
    for part in &parts[start..end] {
        let checked = scrub_part(part)?;
        report.record_part(checked.checked_files, checked.checked_bytes, checked.blocks)?;
    }
    require_unchanged_snapshot(root, &active)?;
    let next_after_part = if end < parts.len() {
        Some(
            parts[end - 1]
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| PartError::InvalidScan("non-UTF8 part scrub cursor".into()))?
                .to_owned(),
        )
    } else {
        None
    };
    Ok(PagedScrubReport {
        generation: active,
        verified: report,
        next_after_part,
    })
}

#[cfg(test)]
mod pinned_snapshot_scrub_pages_tests {
    use super::*;
    use crate::{Column, write_wide_part};
    use std::fs;

    fn test_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rusthouse-scrub-page-{}-{}",
            std::process::id(),
            line!(),
        ))
    }

    #[test]
    fn paged_scrub_agrees_with_complete_report_without_rechecking_parts() {
        let root = test_root();
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for index in 0..5 {
            write_wide_part(
                &root.join(format!("all_{index}_{index}_0")),
                &[Column {
                    name: "id".into(),
                    data_type: "UInt64".into(),
                }],
                &[vec![Some(index.to_string())]],
            )
            .unwrap();
        }
        let complete = scrub_snapshot(&root, 5).unwrap();
        let mut generation = None;
        let mut cursor: Option<String> = None;
        let mut observed = ScrubReport::default();
        for expected_parts in [2, 2, 1] {
            let page =
                scrub_snapshot_page(&root, generation.as_deref(), cursor.as_deref(), 2).unwrap();
            assert_eq!(page.verified.parts, expected_parts);
            observed.parts += page.verified.parts;
            observed.checked_files += page.verified.checked_files;
            observed.checked_bytes += page.verified.checked_bytes;
            observed.blocks += page.verified.blocks;
            generation = Some(page.generation);
            cursor = page.next_after_part;
        }
        assert_eq!(cursor, None);
        assert_eq!(observed, complete);
        assert!(
            scrub_snapshot_page(&root, generation.as_deref(), Some("missing_part"), 1).is_err()
        );
        assert!(scrub_snapshot_page(&root, None, None, 0).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refusing_switched_generations_prevents_stale_cursor_success() {
        let root = test_root().with_extension("switch");
        let _ = fs::remove_dir_all(&root);
        let old = root.join(".rusthouse-versions/g1");
        let new = root.join(".rusthouse-versions/g2");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        for path in [&old, &new] {
            write_wide_part(
                &path.join("all_0_0_0"),
                &[Column {
                    name: "id".into(),
                    data_type: "UInt64".into(),
                }],
                &[vec![Some("7".into())]],
            )
            .unwrap();
        }
        fs::write(root.join(".rusthouse-active"), "g1\n").unwrap();
        let first = scrub_snapshot_page(&root, None, None, 1).unwrap();
        fs::write(root.join(".rusthouse-active"), "g2\n").unwrap();
        assert!(scrub_snapshot_page(&root, Some(&first.generation), None, 1,).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod indexed_scrub_cursor_tests {
    use super::*;

    #[test]
    fn binary_cursor_matches_a_linear_reference_over_maximum_part_count() {
        let parts = (0..16_384)
            .map(|index| std::path::PathBuf::from(format!("part_{index:05}")))
            .collect::<Vec<_>>();
        for index in [0, 1, 127, 255, 256, 4095, 8191, 16_383] {
            let name = format!("part_{index:05}");
            assert_eq!(scrub_cursor_index(&parts, &name).unwrap(), index + 1);
        }
        for invalid in [
            "",
            "../part_00001",
            "part_00001/",
            "part_00001\\x",
            "part_16384",
        ] {
            assert!(scrub_cursor_index(&parts, invalid).is_err());
        }
    }
}

#[cfg(test)]
mod atomic_scrub_report_accounting_tests {
    use super::*;

    #[test]
    fn overflow_in_any_counter_leaves_report_unchanged() {
        let initial = ScrubReport {
            parts: 1,
            checked_files: 2,
            checked_bytes: 3,
            blocks: 4,
        };
        for (files, bytes, blocks, report) in [
            (2, 3, 4, ScrubReport { checked_files: usize::MAX, ..initial.clone() }),
            (2, 3, 4, ScrubReport { checked_bytes: u64::MAX, ..initial.clone() }),
            (2, 3, 4, ScrubReport { blocks: u64::MAX, ..initial.clone() }),
            (2, 3, 4, ScrubReport { parts: usize::MAX, ..initial.clone() }),
        ] {
            let mut current = report.clone();
            assert!(current.record_part(files, bytes, blocks).is_err());
            assert_eq!(current, report);
        }
        let mut valid = initial;
        valid.record_part(2, 3, 4).unwrap();
        assert_eq!(valid.parts, 2);
        assert_eq!(valid.checked_files, 4);
        assert_eq!(valid.checked_bytes, 6);
        assert_eq!(valid.blocks, 8);
    }
}
