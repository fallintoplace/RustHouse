//! Read a frozen directory containing active, non-overlapping wide MergeTree parts.
//! Do not point this at a live upstream table directory: renames during merges
//! make a filesystem directory scan an inconsistent snapshot.
use crate::{Column, NumericColumn, NumericPartReader, PartError, PartMetadata, inspect_part};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MAX_TABLE_PARTS: usize = 10_000;

fn io_error(path: &Path, source: io::Error) -> PartError {
    PartError::Io {
        path: path.to_owned(),
        source,
    }
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
    if !label.starts_with('g')
        || label.len() < 2
        || !label.as_bytes()[1..].iter().all(u8::is_ascii_digit)
    {
        return Err(PartError::InvalidScan(
            "invalid database snapshot generation".into(),
        ));
    }
    let active = root.join(".rusthouse-versions").join(label);
    let kind = fs::symlink_metadata(&active).map_err(|error| io_error(&active, error))?;
    if !kind.file_type().is_dir() {
        return Err(PartError::InvalidScan(
            "active database snapshot is not a directory".into(),
        ));
    }
    Ok(active)
}

// upstream names are partition_minBlock_maxBlock_level[_mutation].
// The partition id may contain underscores; split from the right instead.
fn part_span(name: &str) -> Option<(String, u64, u64)> {
    let tokens: Vec<&str> = name.split('_').collect();
    let candidate = |suffix: usize| {
        if tokens.len() <= suffix {
            return None;
        }
        let index = tokens.len() - suffix;
        let min = tokens[index].parse::<u64>().ok()?;
        let max = tokens[index + 1].parse::<u64>().ok()?;
        let _level = tokens[index + 2].parse::<u64>().ok()?;
        if suffix == 4 {
            let _mutation = tokens[index + 3].parse::<u64>().ok()?;
        }
        let partition = tokens[..index].join("_");
        (min <= max && !partition.is_empty()).then_some((partition, min, max))
    };
    match (candidate(3), candidate(4)) {
        (Some(part), None) | (None, Some(part)) => Some(part),
        // A numeric partition suffix can look like a mutation suffix.
        // Refuse to guess when both parses are plausible.
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
                "table snapshot contains over 10000 parts".into(),
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

fn inspect_parts_metadata(path: &Path, paths: &[PathBuf]) -> Result<PartMetadata, PartError> {
    let first = inspect_part(&paths[0])?;
    let mut rows = 0u64;
    for part in paths {
        let metadata = inspect_part(part)?;
        if metadata.columns != first.columns {
            return Err(PartError::InvalidScan(format!(
                "part schema mismatch in {}",
                part.display()
            )));
        }
        rows = rows
            .checked_add(metadata.rows)
            .ok_or_else(|| PartError::InvalidScan("table row count overflow".into()))?;
    }
    Ok(PartMetadata {
        path: path.to_owned(),
        columns: first.columns,
        rows,
    })
}

/// Keep only one part's streams open at once; the executor sees a single
/// continuous sequence, so GROUP BY, ORDER BY, DISTINCT and LIMIT are global.
pub struct NumericQueryReader {
    parts: Vec<PathBuf>,
    columns: Vec<String>,
    next_part: usize,
    current: Option<NumericPartReader>,
}

impl NumericQueryReader {
    pub fn open(path: &Path, columns: &[String]) -> Result<Self, PartError> {
        let parts = query_parts(path)?;
        let metadata = inspect_parts_metadata(path, &parts)?;
        for column in columns {
            if !metadata.columns.iter().any(|c: &Column| &c.name == column) {
                return Err(PartError::InvalidScan(format!("unknown column: {column}")));
            }
        }
        Ok(Self {
            parts,
            columns: columns.to_owned(),
            next_part: 0,
            current: None,
        })
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
            // Opening validates stream layout and marks before we skip them.
            let mut reader = NumericPartReader::open(&self.parts[self.next_part], &self.columns)?;
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
        loop {
            if let Some(current) = self.current.as_mut() {
                if let Some(batch) = current.next_batch()? {
                    return Ok(Some(batch));
                }
                self.current = None;
            }
            if self.next_part == self.parts.len() {
                return Ok(None);
            }
            self.current = Some(NumericPartReader::open(
                &self.parts[self.next_part],
                &self.columns,
            )?);
            self.next_part += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parses_standard_part_names() {
        assert_eq!(part_span("all_1_2_0"), Some(("all".into(), 1, 2)));
        assert_eq!(part_span("2026_10_1_2_0"), Some(("2026_10".into(), 1, 2)));
        assert_eq!(part_span("all_1_2_0_7"), Some(("all".into(), 1, 2)));
        assert_eq!(part_span("all_3_2_0"), None);
        assert_eq!(part_span("_1_2_0"), None);
        assert_eq!(part_span("all__1_2_0"), Some(("all_".into(), 1, 2)));
        assert_eq!(part_span("all_1_2_0_"), None);
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
