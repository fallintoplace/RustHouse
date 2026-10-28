//! Read a frozen directory containing active, non-overlapping wide MergeTree parts.
//! Do not point this at a live upstream table directory: renames during merges
//! make a filesystem directory scan an inconsistent snapshot.
use crate::{Column, NumericColumn, NumericPartReader, PartError, PartMetadata, inspect_part};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MAX_TABLE_PARTS: usize = 16_384;

fn io_error(path: &Path, source: io::Error) -> PartError {
    PartError::Io {
        path: path.to_owned(),
        source,
    }
}

fn valid_generation_label(label: &str) -> bool {
    label.starts_with('g')
        && label.len() >= 2
        && label.as_bytes()[1..].iter().all(u8::is_ascii_digit)
}

fn require_snapshot_directory(path: &Path, message: &'static str) -> Result<(), PartError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if !metadata.file_type().is_dir() {
        return Err(PartError::InvalidScan(message.into()));
    }
    Ok(())
}

/// Resolve the immutable active generation of a local table.
/// Legacy tables without a pointer use their original parts directory.
pub fn resolve_active_snapshot(root: &Path) -> Result<PathBuf, PartError> {
    let pointer = root.join(".rusthouse-active");
    let metadata = match fs::symlink_metadata(&pointer) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(root.to_owned()),
        Err(error) => return Err(io_error(&pointer, error)),
    };
    if !metadata.file_type().is_file() || metadata.len() > 64 {
        return Err(PartError::InvalidScan(
            "invalid database snapshot pointer".into(),
        ));
    }
    let label = fs::read_to_string(&pointer).map_err(|error| io_error(&pointer, error))?;
    let label = label
        .strip_suffix('\n')
        .ok_or_else(|| PartError::InvalidScan("invalid snapshot pointer format".into()))?;
    if !valid_generation_label(label) {
        return Err(PartError::InvalidScan(
            "invalid database snapshot generation".into(),
        ));
    }
    let versions = root.join(".rusthouse-versions");
    require_snapshot_directory(
        &versions,
        "database snapshot versions root is not a directory",
    )?;
    let active = versions.join(label);
    require_snapshot_directory(&active, "active database snapshot is not a directory")?;
    Ok(active)
}

// upstream names are partition_minBlock_maxBlock_level[_mutation].
// The partition id may contain underscores; split from the right instead.
fn parse_part_span_suffix(name: &str, suffix: usize) -> Option<(&str, u64, u64)> {
    let mut tokens = name.rsplitn(suffix + 1, '_');
    if suffix == 4 {
        let _mutation = tokens.next()?.parse::<u64>().ok()?;
    }
    let _level = tokens.next()?.parse::<u64>().ok()?;
    let max = tokens.next()?.parse::<u64>().ok()?;
    let min = tokens.next()?.parse::<u64>().ok()?;
    let partition = tokens.next()?;
    (min <= max && !partition.is_empty()).then_some((partition, min, max))
}

fn part_span(name: &str) -> Option<(String, u64, u64)> {
    // Preserve the numeric-suffix ambiguity check without allocating two
    // candidate partition strings for each part in a large snapshot.
    match (
        parse_part_span_suffix(name, 3),
        parse_part_span_suffix(name, 4),
    ) {
        (Some((partition, min, max)), None) | (None, Some((partition, min, max))) => {
            Some((partition.to_owned(), min, max))
        }
        _ => None,
    }
}

fn validate_block_ranges(ranges: &mut HashMap<String, Vec<(u64, u64)>>) -> Result<(), PartError> {
    for spans in ranges.values_mut() {
        spans.sort_unstable();
        for pair in spans.windows(2) {
            if pair[0].1 >= pair[1].0 {
                return Err(PartError::InvalidScan(
                    "overlapping part block ranges: provide a frozen active-part snapshot".into(),
                ));
            }
        }
    }
    Ok(())
}

/// List complete active wide parts from an offline snapshot.
/// Refuse overlapping block ranges rather than double-count replaced parts.
pub fn query_parts(path: &Path) -> Result<Vec<PathBuf>, PartError> {
    if path.join("count.txt").exists() || path.join("columns.txt").exists() {
        return Ok(vec![path.to_owned()]);
    }
    let mut parts = Vec::new();
    let mut ranges: HashMap<String, Vec<(u64, u64)>> = HashMap::new();
    for entry in fs::read_dir(path).map_err(|e| io_error(path, e))? {
        let entry = entry.map_err(|e| io_error(path, e))?;
        let file_type = entry.file_type().map_err(|e| io_error(&entry.path(), e))?;
        if !file_type.is_dir() {
            continue; // Symlinks are deliberately not followed.
        }
        let name = entry.file_name();
        // Older local writers staged data inside the active parts directory.
        // Never expose those incomplete, unpublished parts to SQL reads.
        if name
            .to_str()
            .is_some_and(|name| name.starts_with(".pending-"))
        {
            continue;
        }
        let child = entry.path();
        let has_count = child.join("count.txt").exists();
        let has_columns = child.join("columns.txt").exists();
        if !has_count && !has_columns {
            continue; // e.g. detached/, shadow/ or non-part metadata.
        }
        if has_count != has_columns {
            return Err(PartError::InvalidScan(format!(
                "incomplete part metadata in {}",
                child.display()
            )));
        }
        let name = name
            .to_str()
            .ok_or_else(|| PartError::InvalidScan("part directory name is not UTF-8".into()))?;
        let (partition, min, max) = part_span(name).ok_or_else(|| {
            PartError::InvalidScan(format!("not a standard active part name: {name}"))
        })?;
        ranges.entry(partition).or_default().push((min, max));
        parts.push(child);
        if parts.len() > MAX_TABLE_PARTS {
            return Err(PartError::InvalidScan(
                "table snapshot contains over 16384 parts".into(),
            ));
        }
    }
    if parts.is_empty() {
        return Err(PartError::InvalidScan(
            "no active wide MergeTree parts found in snapshot".into(),
        ));
    }
    validate_block_ranges(&mut ranges)?;
    parts.sort_unstable();
    Ok(parts)
}

/// Return consistent metadata for a single part or a frozen table snapshot.
pub fn inspect_query_source(path: &Path) -> Result<PartMetadata, PartError> {
    let paths = query_parts(path)?;
    inspect_parts_metadata(path, &paths)
}

pub(crate) fn inspect_parts_metadata(
    path: &Path,
    paths: &[PathBuf],
) -> Result<PartMetadata, PartError> {
    // Part metadata is immutable for an offline snapshot. For tables with many
    // parts, inspect independent files in parallel without spawning one thread
    // per part. Preserve input order so corruption reports stay deterministic.
    const PARALLEL_THRESHOLD: usize = 32;
    const MAX_WORKERS: usize = 8;
    let workers = std::thread::available_parallelism()
        .map_or(1, |parallelism| parallelism.get())
        .min(MAX_WORKERS);
    let mut metadata = Vec::with_capacity(paths.len());
    if paths.len() >= PARALLEL_THRESHOLD && workers > 1 {
        // A few parts can have much larger metadata than the rest. Claim
        // part indexes dynamically so one slow range does not hold up the
        // whole scan. Retain results by input index for stable diagnostics.
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cursor = AtomicUsize::new(0);
        let mut slots: Vec<Option<Result<PartMetadata, PartError>>> =
            (0..paths.len()).map(|_| None).collect();
        std::thread::scope(|scope| -> Result<(), PartError> {
            let handles = (0..workers)
                .map(|_| {
                    let cursor = &cursor;
                    scope.spawn(move || {
                        let mut completed = Vec::new();
                        loop {
                            let index = cursor.fetch_add(1, Ordering::Relaxed);
                            if index >= paths.len() {
                                break;
                            }
                            completed.push((index, inspect_part(&paths[index])));
                        }
                        completed
                    })
                })
                .collect::<Vec<_>>();
            for handle in handles {
                let completed = handle.join().map_err(|_| {
                    PartError::InvalidScan("snapshot metadata worker panicked".into())
                })?;
                for (index, item) in completed {
                    slots[index] = Some(item);
                }
            }
            Ok(())
        })?;
        for slot in slots {
            metadata.push(slot.expect("every claimed part has one result")?);
        }
    } else {
        for part in paths {
            metadata.push(inspect_part(part)?);
        }
    }

    let first = metadata
        .first()
        .ok_or_else(|| PartError::InvalidScan("snapshot contains no readable parts".into()))?;
    let mut rows = 0u64;
    for (part, item) in paths.iter().zip(&metadata) {
        if item.columns != first.columns {
            return Err(PartError::InvalidScan(format!(
                "part schema mismatch in {}",
                part.display()
            )));
        }
        rows = rows
            .checked_add(item.rows)
            .ok_or_else(|| PartError::InvalidScan("table row count overflow".into()))?;
    }
    Ok(PartMetadata {
        path: path.to_owned(),
        columns: first.columns.clone(),
        rows,
    })
}

/// Reuse one bounded worker across thousands of immutable numeric parts.
/// A single in-flight open avoids thread creation per part without retaining
/// data batches or opening the whole table at once.
struct NumericPartPrefetchWorker {
    requests: Option<std::sync::mpsc::SyncSender<PathBuf>>,
    responses: std::sync::mpsc::Receiver<Result<NumericPartReader, PartError>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl NumericPartPrefetchWorker {
    fn new(columns: std::sync::Arc<[String]>) -> Self {
        let (requests, jobs) = std::sync::mpsc::sync_channel::<PathBuf>(1);
        let (answers, responses) =
            std::sync::mpsc::sync_channel::<Result<NumericPartReader, PartError>>(1);
        let thread = std::thread::spawn(move || {
            while let Ok(path) = jobs.recv() {
                if answers.send(NumericPartReader::open(&path, &columns)).is_err() {
                    break;
                }
            }
        });
        Self {
            requests: Some(requests),
            responses,
            thread: Some(thread),
        }
    }
}

impl Drop for NumericPartPrefetchWorker {
    fn drop(&mut self) {
        // Disconnect requests first so a worker waiting for more jobs exits.
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Keep only one part's streams open at once; the executor sees a single
/// continuous sequence, so GROUP BY, ORDER BY, DISTINCT and LIMIT are global.
pub struct NumericQueryReader {
    parts: Vec<PathBuf>,
    columns: std::sync::Arc<[String]>,
    next_part: usize,
    current: Option<NumericPartReader>,
    // One prefetched part open, never prefetched decoded data batches.
    prefetched: bool,
    worker: Option<NumericPartPrefetchWorker>,
    #[cfg(test)]
    prefetch_worker_starts: usize,
}

impl NumericQueryReader {
    pub fn open(path: &Path, columns: &[String]) -> Result<Self, PartError> {
        let parts = query_parts(path)?;
        let metadata = inspect_parts_metadata(path, &parts)?;
        Self::from_validated_parts(parts, columns, &metadata)
    }

    /// Reuse metadata that was validated for this immutable snapshot by
    /// the query planner, rather than opening every part's metadata twice.
    pub fn open_with_metadata(
        path: &Path,
        columns: &[String],
        metadata: &PartMetadata,
    ) -> Result<Self, PartError> {
        if metadata.path.as_path() != path {
            return Err(PartError::InvalidScan(
                "scan metadata belongs to a different snapshot".into(),
            ));
        }
        let parts = query_parts(path)?;
        Self::from_validated_parts(parts, columns, metadata)
    }

    fn from_validated_parts(
        parts: Vec<PathBuf>,
        columns: &[String],
        metadata: &PartMetadata,
    ) -> Result<Self, PartError> {
        for column in columns {
            if !metadata.columns.iter().any(|c: &Column| &c.name == column) {
                return Err(PartError::InvalidScan(format!("unknown column: {column}")));
            }
        }
        Ok(Self {
            parts,
            columns: std::sync::Arc::from(columns.to_owned()),
            next_part: 0,
            current: None,
            prefetched: false,
            worker: None,
            #[cfg(test)]
            prefetch_worker_starts: 0,
        })
    }

    /// Keep exactly one future part open in the worker's bounded queue.
    fn prefetch_next(&mut self) {
        if self.prefetched || self.next_part >= self.parts.len() {
            return;
        }
        if self.worker.is_none() {
            self.worker = Some(NumericPartPrefetchWorker::new(self.columns.clone()));
            #[cfg(test)]
            { self.prefetch_worker_starts += 1; }
        }
        let path = self.parts[self.next_part].clone();
        let queued = self.worker.as_ref()
            .and_then(|worker| worker.requests.as_ref())
            .is_some_and(|queue| queue.send(path).is_ok());
        if queued {
            self.next_part += 1;
            self.prefetched = true;
        } else {
            self.worker.take();
        }
    }

    fn advance_part(&mut self) -> Result<bool, PartError> {
        let reader = if self.prefetched {
            self.prefetched = false;
            self.worker.as_ref().ok_or_else(|| {
                PartError::InvalidScan("missing numeric part prefetch worker".into())
            })?.responses.recv().map_err(|_| {
                PartError::InvalidScan("numeric snapshot part opener stopped".into())
            })??
        } else if self.next_part < self.parts.len() {
            let reader = NumericPartReader::open(
                &self.parts[self.next_part], &self.columns,
            )?;
            self.next_part += 1;
            reader
        } else {
            return Ok(false);
        };
        self.current = Some(reader);
        self.prefetch_next();
        Ok(true)
    }

    /// Skip as much of an unfiltered OFFSET as possible using part/mark
    /// boundaries. No predicate or DISTINCT rows may be discarded this way.
    /// The result is the number of complete rows skipped, so the executor can
    /// still discard a remainder in the first decoded batch.
    pub fn skip_prefix_rows(&mut self, max_rows: usize) -> Result<usize, PartError> {
        if self.next_part != 0 || self.current.is_some() {
            return Err(PartError::InvalidScan(
                "OFFSET mark skipping requires a fresh reader".into(),
            ));
        }
        let mut skipped = 0usize;
        while skipped < max_rows && self.next_part < self.parts.len() {
            // Immutable part row counts are validated by the snapshot planner.
            // Skipping a complete part never needs to open its wide streams or
            // decode thousands of mark entries.
            let part = &self.parts[self.next_part];
            let metadata = inspect_part(part)?;
            if let Ok(rows) = usize::try_from(metadata.rows) {
                if rows <= max_rows - skipped {
                    skipped += rows;
                    self.next_part += 1;
                    continue;
                }
            }
            let mut reader = NumericPartReader::open(part, &self.columns)?;
            self.next_part += 1;
            let amount = reader.skip_complete_marks(max_rows - skipped);
            skipped += amount;
            if u64::try_from(amount).ok() != Some(reader.rows()) {
                self.current = Some(reader);
                break;
            }
        }
        Ok(skipped)
    }

    pub fn next_batch(&mut self) -> Result<Option<Vec<NumericColumn>>, PartError> {
        // OFFSET may have already installed a partially skipped part.
        if self.current.is_some() {
            self.prefetch_next();
        }
        loop {
            if let Some(current) = self.current.as_mut() {
                if let Some(batch) = current.next_batch()? {
                    return Ok(Some(batch));
                }
                self.current = None;
            }
            if !self.advance_part()? {
                return Ok(None);
            }
        }
    }
}

impl Drop for NumericQueryReader {
    fn drop(&mut self) {
        // Wait for any active open, including when LIMIT exits early.
        self.worker.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_snapshot_versions_directory() {
        use std::os::unix::fs::symlink;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rusthouse-versions-link-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("other")).unwrap();
        fs::create_dir(root.join("other/g1")).unwrap();
        fs::write(root.join(".rusthouse-active"), b"g1\n").unwrap();
        symlink("other", root.join(".rusthouse-versions")).unwrap();

        assert!(matches!(
            resolve_active_snapshot(&root),
            Err(PartError::InvalidScan(message))
                if message == "database snapshot versions root is not a directory"
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ignores_unpublished_pending_local_parts() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

        let root = std::env::temp_dir().join(format!(
            "rusthouse-pending-part-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let active = root.join("all_0_0_0");
        let pending = root.join(".pending-123-0");
        fs::create_dir_all(&active).unwrap();
        fs::create_dir(&pending).unwrap();
        for path in [&active, &pending] {
            fs::write(path.join("count.txt"), "0\n").unwrap();
            fs::write(
                path.join("columns.txt"),
                "columns format version: 1\n1 columns:\n`id` UInt64\n",
            )
            .unwrap();
        }
        assert_eq!(query_parts(&root).unwrap(), vec![active]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prefetches_numeric_part_open_without_reordering_batches_or_offset() {
        use crate::write_wide_part;
        let root = std::env::temp_dir().join(format!(
            "rusthouse-numeric-part-prefetch-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let columns = vec![Column { name: "id".into(), data_type: "UInt64".into() }];
        for index in 0..24u64 {
            write_wide_part(
                &root.join(format!("all_{index}_{index}_0")),
                &columns,
                &[vec![Some(index.to_string())]],
            ).unwrap();
        }
        let mut scan = NumericQueryReader::open(&root, &["id".to_owned()]).unwrap();
        let mut seen = Vec::new();
        while let Some(batch) = scan.next_batch().unwrap() {
            if let NumericColumn::UInt64(values) = &batch[0] {
                seen.extend_from_slice(values);
            } else {
                panic!("expected UInt64 batch");
            }
        }
        assert_eq!(seen, (0..24u64).collect::<Vec<_>>());
        assert_eq!(scan.prefetch_worker_starts, 1);
        drop(scan);

        let mut paged = NumericQueryReader::open(&root, &["id".to_owned()]).unwrap();
        assert_eq!(paged.skip_prefix_rows(18).unwrap(), 18);
        let mut rest = Vec::new();
        while let Some(batch) = paged.next_batch().unwrap() {
            if let NumericColumn::UInt64(values) = &batch[0] {
                rest.extend_from_slice(values);
            }
        }
        assert_eq!(rest, (18..24u64).collect::<Vec<_>>());
        drop(paged);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inspects_many_immutable_parts_and_rejects_one_incompatible_schema() {
        use crate::write_wide_part;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT_TEST: AtomicUsize = AtomicUsize::new(0);

        let root = std::env::temp_dir().join(format!(
            "rusthouse-parallel-metadata-{}-{}",
            std::process::id(),
            NEXT_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let columns = vec![Column {
            name: "id".into(),
            data_type: "UInt64".into(),
        }];
        for index in 1..=40 {
            let part = root.join(format!("all_{index}_{index}_0"));
            write_wide_part(&part, &columns, &[vec![Some(index.to_string())]]).unwrap();
        }
        let metadata = inspect_query_source(&root).unwrap();
        assert_eq!(metadata.rows, 40);
        assert_eq!(metadata.columns, columns);
        let mut reader =
            NumericQueryReader::open_with_metadata(&root, &["id".into()], &metadata).unwrap();
        let mut inspected = 0;
        while let Some(batch) = reader.next_batch().unwrap() {
            inspected += batch[0].len();
        }
        assert_eq!(inspected, 40);
        let mut paged =
            NumericQueryReader::open_with_metadata(&root, &["id".into()], &metadata).unwrap();
        assert_eq!(paged.skip_prefix_rows(39).unwrap(), 39);
        let tail = paged.next_batch().unwrap().unwrap();
        assert!(matches!(&tail[0], NumericColumn::UInt64(values) if values == &[40]));
        assert!(paged.next_batch().unwrap().is_none());
        assert!(matches!(
            NumericQueryReader::open_with_metadata(
                &root.join("other-snapshot"),
                &["id".into()],
                &metadata,
            ),
            Err(PartError::InvalidScan(_))
        ));

        let changed = root.join("all_40_40_0");
        fs::remove_dir_all(&changed).unwrap();
        write_wide_part(
            &changed,
            &[Column {
                name: "other".into(),
                data_type: "UInt64".into(),
            }],
            &[vec![Some("40".into())]],
        )
        .unwrap();
        assert!(matches!(
            inspect_query_source(&root),
            Err(PartError::InvalidScan(message)) if message.contains("schema mismatch")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_standard_part_names() {
        assert_eq!(part_span("all_1_2_0"), Some(("all".into(), 1, 2)));
        assert_eq!(part_span("2026_10_1_2_0"), Some(("2026_10".into(), 1, 2)));
        assert_eq!(part_span("all_1_2_0_7"), Some(("all".into(), 1, 2)));
        assert_eq!(part_span("all_3_2_0"), None);
        assert_eq!(part_span("_1_2_0"), None);
        assert_eq!(part_span("all__1_2_0"), Some(("all_".into(), 1, 2)));
        assert_eq!(part_span("all_1_2_0_"), None);
        assert_eq!(part_span("all_1_2_3_4"), None);
        assert_eq!(
            part_span("region_west_5_9_0"),
            Some(("region_west".into(), 5, 9))
        );
        assert_eq!(
            part_span("region_west_5_9_0_12"),
            Some(("region_west".into(), 5, 9))
        );
    }
    #[test]
    fn checks_overlapping_block_ranges_independently_of_directory_order() {
        let mut ranges = HashMap::from([("all".to_owned(), vec![(10, 12), (1, 3), (3, 5)])]);
        assert!(matches!(
            validate_block_ranges(&mut ranges),
            Err(PartError::InvalidScan(_))
        ));

        let mut ranges = HashMap::from([("all".to_owned(), vec![(10, 12), (1, 3), (4, 5)])]);
        validate_block_ranges(&mut ranges).unwrap();
    }
}
