mod range_keys;

use self::range_keys::*;

use crate::{CompressedBlockReader, PartError};
use std::fs::{self, File};
use std::io::Read;
use std::ops::{Bound, Range};
use std::path::Path;

const MAX_INDEX_BYTES: usize = 256 * 1024 * 1024;
const MAX_INDEX_FIELDS: usize = 4096;

#[cfg(test)]
mod compressed_primary_index_identity_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SERIAL: AtomicUsize = AtomicUsize::new(0);

    fn temporary_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rusthouse-primary-compressed-{}-{}.cidx",
            std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[cfg(unix)]
    #[test]
    fn compressed_primary_index_refuses_symlinked_source() {
        use std::os::unix::fs::symlink;
        let source = temporary_path();
        let alias = source.with_extension("alias.cidx");
        fs::write(&source, b"not a compressed index").unwrap();
        symlink(&source, &alias).unwrap();
        assert!(matches!(read_fixed_primary_index(&alias, &[8]),
            Err(PartError::Io { .. })));
        fs::remove_file(&alias).unwrap();
        fs::remove_file(&source).unwrap();
    }

    #[test]
    fn metadata_change_detector_rejects_changed_file_length() {
        let path = temporary_path();
        fs::write(&path, b"checkpoint_000000").unwrap();
        let before = fs::metadata(&path).unwrap();
        // A size change is always a detectable failure, even where a
        // filesystem has coarse timestamp precision.
        fs::write(&path, b"checkpoint_000000changed").unwrap();
        let after = fs::metadata(&path).unwrap();
        assert!(!raw_index_unchanged(&before, &after));
        fs::remove_file(path).unwrap();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedPrimaryIndex {
    data: Vec<u8>,
    field_offsets: Vec<usize>,
    row_width: usize,
}

impl FixedPrimaryIndex {
    pub fn len(&self) -> usize {
        self.data.len() / self.row_width
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn field(&self, mark: usize, field: usize) -> Option<&[u8]> {
        let start = *self.field_offsets.get(field)?;
        let end = *self.field_offsets.get(field.checked_add(1)?)?;
        let base = mark.checked_mul(self.row_width)?;
        self.data
            .get(base.checked_add(start)?..base.checked_add(end)?)
    }
}

/// Conservative granule range for an ascending fixed-width index.
///
/// A mark stores the first key of a granule. The predecessor of the first
/// matching mark can also contain matching rows, so range selection must
/// retain it. Callers still apply the exact row predicate.
/// Decode a narrow physical key without sign loss or host-endian assumptions.
fn narrow_unsigned_mark_key(
    index: &FixedPrimaryIndex, mark: usize, field: usize,
) -> Option<u64> {
    match index.field(mark, field)? {
        [a] => Some(u64::from(*a)),
        [a, b] => Some(u64::from(u16::from_le_bytes([*a, *b]))),
        [a, b, c, d] => Some(u64::from(u32::from_le_bytes([*a, *b, *c, *d]))),
        _ => None,
    }
}

fn narrow_signed_mark_key(
    index: &FixedPrimaryIndex, mark: usize, field: usize,
) -> Option<i64> {
    match index.field(mark, field)? {
        [a] => Some(i64::from(i8::from_le_bytes([*a]))),
        [a, b] => Some(i64::from(i16::from_le_bytes([*a, *b]))),
        [a, b, c, d] => Some(i64::from(i32::from_le_bytes([*a, *b, *c, *d]))),
        _ => None,
    }
}

/// Conservatively select mark ranges for UInt8, UInt16, or UInt32 key fields.
/// Bounds use u64 so callers can express values beyond the physical type's
/// range without truncation or wraparound.
pub fn candidate_narrow_unsigned_mark_range(
    index: &FixedPrimaryIndex,
    field: usize,
    lower: Bound<u64>,
    upper: Bound<u64>,
) -> Option<Range<usize>> {
    candidate_mark_range(index.len(), |mark| narrow_unsigned_mark_key(index, mark, field), lower, upper)
}

/// Select mark ranges for signed Int8, Int16, or Int32 primary-key fields.
pub fn candidate_narrow_signed_mark_range(
    index: &FixedPrimaryIndex,
    field: usize,
    lower: Bound<i64>,
    upper: Bound<i64>,
) -> Option<Range<usize>> {
    candidate_mark_range(index.len(), |mark| narrow_signed_mark_key(index, mark, field), lower, upper)
}

/// Merge disjunctive intervals for UInt8/16/32 primary-key fields.
pub fn candidate_narrow_unsigned_mark_ranges(
    index: &FixedPrimaryIndex,
    field: usize,
    intervals: &[(Bound<u64>, Bound<u64>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(index.len(), |mark| narrow_unsigned_mark_key(index, mark, field), intervals)
}

/// Merge disjunctive intervals for Int8/16/32 primary-key fields.
pub fn candidate_narrow_signed_mark_ranges(
    index: &FixedPrimaryIndex,
    field: usize,
    intervals: &[(Bound<i64>, Bound<i64>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(index.len(), |mark| narrow_signed_mark_key(index, mark, field), intervals)
}

/// Returns a conservative mark range for an ascending UInt64 key.
/// Invalid or unsorted bounded indexes return None. Fully unbounded scans
/// return every granule without reading or validating individual keys.
pub fn candidate_u64_mark_range(
    index: &FixedPrimaryIndex,
    field: usize,
    lower: Bound<u64>,
    upper: Bound<u64>,
) -> Option<Range<usize>> {
    candidate_mark_range(
        index.len(),
        |mark| u64_mark_key(index, mark, field),
        lower,
        upper,
    )
}

/// Returns a conservative mark range for an ascending Int64 key.
///
/// Signed comparisons use the decoded Int64 values rather than their raw
/// little-endian byte order. This is only a planning primitive: callers must
/// still evaluate the predicate for candidate rows.
pub fn candidate_i64_mark_range(
    index: &FixedPrimaryIndex,
    field: usize,
    lower: Bound<i64>,
    upper: Bound<i64>,
) -> Option<Range<usize>> {
    candidate_mark_range(
        index.len(),
        |mark| i64_mark_key(index, mark, field),
        lower,
        upper,
    )
}

fn u64_pair_mark_key(
    index: &FixedPrimaryIndex,
    mark: usize,
    first: usize,
    second: usize,
) -> Option<(u64, u64)> {
    Some((
        u64_mark_key(index, mark, first)?,
        u64_mark_key(index, mark, second)?,
    ))
}

fn i64_pair_mark_key(
    index: &FixedPrimaryIndex,
    mark: usize,
    first: usize,
    second: usize,
) -> Option<(i64, i64)> {
    Some((
        i64_mark_key(index, mark, first)?,
        i64_mark_key(index, mark, second)?,
    ))
}

/// Plan conservative lexicographic granules for two UInt64 primary-key fields.
/// The second key can reset when the first key changes; validating either
/// column independently would incorrectly disable composite index pruning.
pub fn candidate_u64_pair_mark_range(
    index: &FixedPrimaryIndex,
    first: usize,
    second: usize,
    lower: Bound<(u64, u64)>,
    upper: Bound<(u64, u64)>,
) -> Option<Range<usize>> {
    candidate_mark_range(index.len(), |mark| u64_pair_mark_key(index, mark, first, second), lower, upper)
}

/// Plan conservative lexicographic granules for two signed Int64 key fields.
pub fn candidate_i64_pair_mark_range(
    index: &FixedPrimaryIndex,
    first: usize,
    second: usize,
    lower: Bound<(i64, i64)>,
    upper: Bound<(i64, i64)>,
) -> Option<Range<usize>> {
    candidate_mark_range(index.len(), |mark| i64_pair_mark_key(index, mark, first, second), lower, upper)
}

/// Union disjunctive lexicographic UInt64 tuple intervals.
pub fn candidate_u64_pair_mark_ranges(
    index: &FixedPrimaryIndex,
    first: usize,
    second: usize,
    intervals: &[(Bound<(u64, u64)>, Bound<(u64, u64)>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(index.len(), |mark| u64_pair_mark_key(index, mark, first, second), intervals)
}

/// Union disjunctive lexicographic Int64 tuple intervals.
pub fn candidate_i64_pair_mark_ranges(
    index: &FixedPrimaryIndex,
    first: usize,
    second: usize,
    intervals: &[(Bound<(i64, i64)>, Bound<(i64, i64)>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(index.len(), |mark| i64_pair_mark_key(index, mark, first, second), intervals)
}

/// Plan and coalesce candidate granules for UInt64 IN and OR predicates.
pub fn candidate_u64_mark_ranges(
    index: &FixedPrimaryIndex,
    field: usize,
    intervals: &[(Bound<u64>, Bound<u64>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(
        index.len(),
        |mark| u64_mark_key(index, mark, field),
        intervals,
    )
}

/// Plan and coalesce candidate granules for signed Int64 predicates.
pub fn candidate_i64_mark_ranges(
    index: &FixedPrimaryIndex,
    field: usize,
    intervals: &[(Bound<i64>, Bound<i64>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(
        index.len(),
        |mark| i64_mark_key(index, mark, field),
        intervals,
    )
}

/// Conservatively prune a sorted UInt128 primary key, retaining a
/// predecessor granule so row-level predicates remain exact.
pub fn candidate_u128_mark_range(
    index: &FixedPrimaryIndex,
    field: usize,
    lower: Bound<u128>,
    upper: Bound<u128>,
) -> Option<Range<usize>> {
    candidate_mark_range(index.len(), |mark| u128_mark_key(index, mark, field), lower, upper)
}

/// Conservatively prune a signed Int128 primary key.
pub fn candidate_i128_mark_range(
    index: &FixedPrimaryIndex,
    field: usize,
    lower: Bound<i128>,
    upper: Bound<i128>,
) -> Option<Range<usize>> {
    candidate_mark_range(index.len(), |mark| i128_mark_key(index, mark, field), lower, upper)
}

/// Coalesce disjunctive UInt128 primary key intervals into mark ranges.
pub fn candidate_u128_mark_ranges(
    index: &FixedPrimaryIndex,
    field: usize,
    intervals: &[(Bound<u128>, Bound<u128>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(index.len(), |mark| u128_mark_key(index, mark, field), intervals)
}

/// Coalesce disjunctive Int128 primary key intervals into mark ranges.
pub fn candidate_i128_mark_ranges(
    index: &FixedPrimaryIndex,
    field: usize,
    intervals: &[(Bound<i128>, Bound<i128>)],
) -> Option<Vec<Range<usize>>> {
    candidate_mark_ranges(index.len(), |mark| i128_mark_key(index, mark, field), intervals)
}

pub fn read_fixed_primary_index(
    path: impl AsRef<Path>,
    field_widths: &[usize],
) -> Result<FixedPrimaryIndex, PartError> {
    let path = path.as_ref();
    let field_offsets = checked_field_offsets(path, field_widths)?;
    let row_width = *field_offsets.last().unwrap();

    let data = match path.extension().and_then(|extension| extension.to_str()) {
        Some("idx") => read_raw(path)?,
        Some("cidx") => read_compressed(path)?,
        _ => return Err(invalid(path, "expected .idx or .cidx")),
    };
    if data.len() % row_width != 0 {
        return Err(invalid(path, "trailing incomplete key tuple"));
    }
    Ok(FixedPrimaryIndex {
        data,
        field_offsets,
        row_width,
    })
}

fn checked_field_offsets(path: &Path, field_widths: &[usize]) -> Result<Vec<usize>, PartError> {
    if field_widths.is_empty() {
        return Err(invalid(path, "at least one key field is required"));
    }
    if field_widths.len() > MAX_INDEX_FIELDS {
        return Err(invalid(path, "too many primary-key fields"));
    }
    let mut field_offsets = Vec::with_capacity(field_widths.len() + 1);
    field_offsets.push(0usize);
    for &width in field_widths {
        if width == 0 {
            return Err(invalid(path, "key field width must be positive"));
        }
        let offset = field_offsets
            .last()
            .unwrap()
            .checked_add(width)
            .filter(|size| *size <= MAX_INDEX_BYTES)
            .ok_or_else(|| invalid(path, "key width exceeds the 256 MiB limit"))?;
        field_offsets.push(offset);
    }
    Ok(field_offsets)
}

// A frozen primary index must retain the same identity and state across a
// read. A same-size in-place rewrite can otherwise yield a torn key index.
fn raw_index_unchanged(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    if !after.file_type().is_file()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec()
    }
    #[cfg(not(unix))]
    true
}

fn read_raw(path: &Path) -> Result<Vec<u8>, PartError> {
    let original = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !original.file_type().is_file() {
        return Err(invalid(path, "primary index must be a regular file"));
    }
    let file = File::open(path).map_err(|source| io_error(path, source))?;
    let opened = file.metadata().map_err(|source| io_error(path, source))?;
    if !raw_index_unchanged(&original, &opened) {
        return Err(invalid(path, "primary index changed before read"));
    }
    let size = opened.len();
    if size > MAX_INDEX_BYTES as u64 {
        return Err(invalid(path, "index exceeds the 256 MiB limit"));
    }
    // The checked file size is known. Avoid repeated growth for large raw indexes.
    let mut data = Vec::with_capacity(size as usize);
    (&file)
        .take(MAX_INDEX_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|source| io_error(path, source))?;
    if data.len() > MAX_INDEX_BYTES {
        return Err(invalid(path, "index exceeds the 256 MiB limit"));
    }
    ensure_exact_primary_index_length(path, size, data.len())?;
    // Check the open inode and its pathname after the read, not just before.
    let finished = file.metadata().map_err(|source| io_error(path, source))?;
    let current = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !raw_index_unchanged(&opened, &finished) || !raw_index_unchanged(&opened, &current) {
        return Err(invalid(path, "primary index changed during read"));
    }
    Ok(data)
}

fn read_compressed(path: &Path) -> Result<Vec<u8>, PartError> {
    let baseline = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    let mut blocks = CompressedBlockReader::open(path)?;
    let opened = blocks.opened_metadata().map_err(|source| io_error(path, source))?;
    if !raw_index_unchanged(&baseline, &opened) {
        return Err(invalid(path, "compressed primary index changed before read"));
    }
    let mut data = Vec::new();
    let mut scratch = Vec::new();
    while let Some(block) = blocks.next_decoded_reusing(std::mem::take(&mut scratch)) {
        let block = block?;
        if block.data.len() > MAX_INDEX_BYTES - data.len() {
            return Err(invalid(path, "index exceeds the 256 MiB limit"));
        }
        data.extend_from_slice(&block.data);
        scratch = block.data;
    }
    // Individual compressed blocks have checksums, but a concurrent writer
    // could replace an entire file between two independently valid blocks.
    // Reject mixed-generation indexes before returning any primary keys.
    let finished = blocks.opened_metadata().map_err(|source| io_error(path, source))?;
    let current = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !raw_index_unchanged(&opened, &finished) || !raw_index_unchanged(&opened, &current) {
        return Err(invalid(path, "compressed primary index changed during read"));
    }
    Ok(data)
}

fn io_error(path: &Path, source: std::io::Error) -> PartError {
    PartError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn invalid(path: &Path, reason: &'static str) -> PartError {
    PartError::InvalidPrimaryIndex {
        path: path.to_path_buf(),
        reason,
    }
}

fn ensure_exact_primary_index_length(
    path: &Path,
    opened_size: u64,
    read_size: usize,
) -> Result<(), PartError> {
    if read_size as u64 != opened_size {
        return Err(invalid(path, "primary index length changed during read"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn conservative_u64_pruning_never_drops_a_matching_granule() {
        let mut data = Vec::new();
        for value in [10u64, 20, 30, 40] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        let index = FixedPrimaryIndex {
            data,
            field_offsets: vec![0, 8],
            row_width: 8,
        };
        // Every granule begins with its index key but can contain later keys.
        let rows = [10..20, 20..30, 30..40, 40..50];
        for lower in 0..60 {
            for upper in lower..60 {
                let candidate = candidate_u64_mark_range(
                    &index,
                    0,
                    Bound::Included(lower),
                    Bound::Included(upper),
                )
                .unwrap();
                for (mark, granule) in rows.iter().enumerate() {
                    if granule
                        .clone()
                        .any(|value| value >= lower && value <= upper)
                    {
                        assert!(
                            candidate.contains(&mark),
                            "query [{lower}, {upper}] skipped matching mark {mark}"
                        );
                    }
                }
            }
        }
        assert_eq!(
            candidate_u64_mark_range(&index, 0, Bound::Included(21), Bound::Excluded(30),),
            Some(1..2)
        );
        assert_eq!(
            candidate_u64_mark_range(&index, 0, Bound::Unbounded, Bound::Unbounded),
            Some(0..4)
        );
    }

    #[test]
    fn signed_i64_pruning_keeps_matching_granules() {
        let keys = [-10i64, -5, 0, 5];
        let index = FixedPrimaryIndex {
            data: keys.into_iter().flat_map(i64::to_le_bytes).collect(),
            field_offsets: vec![0, 8],
            row_width: 8,
        };
        let rows = [[-10, -9], [-5, -4], [0, 1], [5, 6]];
        for lower in -12..8 {
            for upper in lower..8 {
                let candidate = candidate_i64_mark_range(
                    &index,
                    0,
                    Bound::Included(lower),
                    Bound::Included(upper),
                )
                .unwrap();
                for (mark, granule) in rows.iter().enumerate() {
                    if granule
                        .iter()
                        .any(|value| *value >= lower && *value <= upper)
                    {
                        assert!(
                            candidate.contains(&mark),
                            "query [{lower}, {upper}] skipped signed mark {mark}"
                        );
                    }
                }
            }
        }
        assert_eq!(
            candidate_i64_mark_range(&index, 0, Bound::Included(-3), Bound::Excluded(5)),
            Some(1..3)
        );
    }

    #[test]
    fn signed_i64_pruning_handles_duplicates_and_extreme_keys() {
        let index = FixedPrimaryIndex {
            data: [i64::MIN, -10, -10, 0, 9, i64::MAX]
                .into_iter()
                .flat_map(i64::to_le_bytes)
                .collect(),
            field_offsets: vec![0, 8],
            row_width: 8,
        };
        assert_eq!(
            candidate_i64_mark_range(&index, 0, Bound::Included(-9), Bound::Included(0)),
            Some(2..4)
        );
        assert_eq!(
            candidate_i64_mark_range(&index, 0, Bound::Included(i64::MIN), Bound::Excluded(-10)),
            Some(0..1)
        );
        assert_eq!(
            candidate_i64_mark_range(&index, 0, Bound::Unbounded, Bound::Included(i64::MAX)),
            Some(0..6)
        );
        assert_eq!(
            candidate_i64_mark_range(&index, 1, Bound::Unbounded, Bound::Unbounded),
            Some(0..6)
        );
        let unsorted = FixedPrimaryIndex {
            data: [0i64, -1].into_iter().flat_map(i64::to_le_bytes).collect(),
            field_offsets: vec![0, 8],
            row_width: 8,
        };
        assert_eq!(
            candidate_i64_mark_range(&unsorted, 0, Bound::Unbounded, Bound::Unbounded),
            Some(0..2)
        );
    }

    #[test]
    fn invalid_primary_key_layout_disables_pruning() {
        let index = FixedPrimaryIndex {
            data: [20u64, 10].into_iter().flat_map(u64::to_le_bytes).collect(),
            field_offsets: vec![0, 8],
            row_width: 8,
        };
        assert_eq!(
            candidate_u64_mark_range(&index, 0, Bound::Unbounded, Bound::Unbounded),
            Some(0..2)
        );
        assert_eq!(
            candidate_u64_mark_range(&index, 1, Bound::Unbounded, Bound::Unbounded),
            Some(0..2)
        );
    }

    #[test]
    fn batches_and_coalesces_primary_index_candidate_ranges() {
        let index = FixedPrimaryIndex {
            data: [10u64, 20, 30, 40, 50, 60]
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect(),
            field_offsets: vec![0, 8],
            row_width: 8,
        };
        assert_eq!(
            candidate_u64_mark_ranges(
                &index,
                0,
                &[
                    (Bound::Included(51), Bound::Included(52)),
                    (Bound::Included(11), Bound::Included(12)),
                    (Bound::Included(12), Bound::Included(15)),
                ]
            ),
            Some(vec![0..1, 4..5])
        );
        assert_eq!(candidate_u64_mark_ranges(&index, 0, &[]), Some(vec![]));
        assert_eq!(
            candidate_u64_mark_ranges(
                &index,
                0,
                &[
                    (Bound::Included(11), Bound::Included(45)),
                    (Bound::Included(31), Bound::Included(55)),
                ]
            ),
            Some(std::iter::once(0..5).collect::<Vec<_>>())
        );
        let signed = FixedPrimaryIndex {
            data: [-20i64, -10, 0, 10, 20]
                .into_iter()
                .flat_map(i64::to_le_bytes)
                .collect(),
            field_offsets: vec![0, 8],
            row_width: 8,
        };
        assert_eq!(
            candidate_i64_mark_ranges(
                &signed,
                0,
                &[
                    (Bound::Included(-9), Bound::Included(-1)),
                    (Bound::Included(11), Bound::Included(19)),
                ]
            ),
            Some(vec![1..2, 3..4])
        );
        let unsorted = FixedPrimaryIndex {
            data: [30u64, 20].into_iter().flat_map(u64::to_le_bytes).collect(),
            field_offsets: vec![0, 8],
            row_width: 8,
        };
        assert!(
            candidate_u64_mark_ranges(&unsorted, 0, &[(Bound::Unbounded, Bound::Unbounded)])
                == Some(std::iter::once(0..2).collect())
        );
    }

    #[test]
    fn checks_primary_key_field_layout_before_io() {
        let path = Path::new("missing.idx");
        assert_eq!(checked_field_offsets(path, &[1, 2]).unwrap(), vec![0, 1, 3]);
        for widths in [&[][..], &[0][..], &[MAX_INDEX_BYTES, 1][..]] {
            assert!(matches!(
                checked_field_offsets(path, widths),
                Err(PartError::InvalidPrimaryIndex { .. })
            ));
        }
    }

    fn temporary_file(
        extension: &str,
        data: &[u8],
        widths: &[usize],
    ) -> Result<FixedPrimaryIndex, PartError> {
        let path = temporary_path(extension);
        std::fs::write(&path, data).unwrap();
        let result = read_fixed_primary_index(&path, widths);
        std::fs::remove_file(&path).unwrap();
        result
    }

    fn temporary_path(extension: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rusthouse-primary-{}-{}.{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed),
            extension
        ))
    }

    fn checked_block(method: u8, data: &[u8]) -> Vec<u8> {
        let payload = match method {
            0x02 => data.to_vec(),
            0x82 => lz4_flex::block::compress(data),
            0x90 => zstd::bulk::compress(data, 3).unwrap(),
            _ => unreachable!(),
        };
        let mut encoded = vec![method];
        encoded.extend_from_slice(&(9u32 + payload.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&(data.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&payload);
        let mut block = u128::from(ch_cityhash102::cityhash128(&encoded))
            .to_le_bytes()
            .to_vec();
        block.extend_from_slice(&encoded);
        block
    }

    #[test]
    fn reads_uncompressed_fixed_width_tuple_fields() {
        let mut data = Vec::new();
        for (first, second, last) in [(12u32, 99u64, &b"abc"[..]), (42u32, u64::MAX, &b"xyz"[..])] {
            data.extend_from_slice(&first.to_le_bytes());
            data.extend_from_slice(&second.to_le_bytes());
            data.extend_from_slice(last);
        }
        let index = temporary_file("idx", &data, &[4, 8, 3]).unwrap();
        assert_eq!(index.len(), 2);
        assert!(!index.is_empty());
        assert_eq!(index.field(0, 0), Some(&12u32.to_le_bytes()[..]));
        assert_eq!(index.field(1, 1), Some(&u64::MAX.to_le_bytes()[..]));
        assert_eq!(index.field(0, 2), Some(&b"abc"[..]));
        assert_eq!(index.field(1, 2), Some(&b"xyz"[..]));
        assert_eq!(index.field(2, 0), None);
        assert_eq!(index.field(0, 3), None);
        assert_eq!(index.field(usize::MAX, 0), None);
    }

    #[test]
    fn reads_compressed_tuples_across_checked_blocks() {
        let mut raw = Vec::new();
        for (key, value) in [(7u32, 123u64), (14u32, 456u64)] {
            raw.extend_from_slice(&key.to_le_bytes());
            raw.extend_from_slice(&value.to_le_bytes());
        }
        let mut data = checked_block(0x90, &raw[..5]);
        data.extend(checked_block(0x82, &raw[5..13]));
        data.extend(checked_block(0x02, &raw[13..]));
        let index = temporary_file("cidx", &data, &[4, 8]).unwrap();
        assert_eq!(index.len(), 2);
        assert_eq!(index.field(0, 1), Some(&123u64.to_le_bytes()[..]));
        assert_eq!(index.field(1, 0), Some(&14u32.to_le_bytes()[..]));
        assert_eq!(index.field(1, 1), Some(&456u64.to_le_bytes()[..]));
    }

    #[test]
    fn rejects_bad_checksums_and_partial_records() {
        let mut corrupt = checked_block(0x90, &7u64.to_le_bytes());
        corrupt[0] ^= 1;
        assert!(matches!(
            temporary_file("cidx", &corrupt, &[8]),
            Err(PartError::InvalidCompressedBlock {
                reason: "checksum mismatch",
                ..
            })
        ));
        let mut short = checked_block(0x02, &7u64.to_le_bytes()[..3]);
        short.extend(checked_block(0x82, &[4, 5]));
        assert!(matches!(
            temporary_file("cidx", &short, &[8]),
            Err(PartError::InvalidPrimaryIndex {
                reason: "trailing incomplete key tuple",
                ..
            })
        ));
        assert!(matches!(
            temporary_file("idx", &[1, 2, 3], &[4]),
            Err(PartError::InvalidPrimaryIndex {
                reason: "trailing incomplete key tuple",
                ..
            })
        ));
    }

    #[test]
    fn rejects_missing_widths_and_unsupported_formats() {
        for widths in [&[][..], &[0usize][..], &[usize::MAX][..]] {
            assert!(matches!(
                temporary_file("idx", &[], widths),
                Err(PartError::InvalidPrimaryIndex { .. })
            ));
        }
        assert!(matches!(
            temporary_file("idx", &[], &vec![1; MAX_INDEX_FIELDS + 1]),
            Err(PartError::InvalidPrimaryIndex {
                reason: "too many primary-key fields",
                ..
            })
        ));
        assert!(matches!(
            temporary_file("mrk2", &[], &[8]),
            Err(PartError::InvalidPrimaryIndex {
                reason: "expected .idx or .cidx",
                ..
            })
        ));
    }

    #[test]
    fn accepts_empty_indexes_and_reports_missing_files() {
        let index = temporary_file("idx", &[], &[8]).unwrap();
        assert_eq!(index.len(), 0);
        assert!(index.is_empty());
        let path = temporary_path("cidx");
        assert!(matches!(
            read_fixed_primary_index(&path, &[8]),
            Err(PartError::Io { .. })
        ));
    }

    #[test]
    fn rejects_oversized_sparse_indexes_before_reading() {
        let path = temporary_path("idx");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_INDEX_BYTES as u64 + 1).unwrap();
        assert!(matches!(
            read_fixed_primary_index(&path, &[8]),
            Err(PartError::InvalidPrimaryIndex {
                reason: "index exceeds the 256 MiB limit",
                ..
            })
        ));
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn searches_large_packed_index_without_changing_predecessor_bounds() {
        let data: Vec<u8> = (0..8_192u64).flat_map(u64::to_le_bytes).collect();
        let index = FixedPrimaryIndex {
            data,
            field_offsets: vec![0, 8],
            row_width: 8,
        };
        assert_eq!(
            candidate_u64_mark_range(&index, 0, Bound::Included(512), Bound::Included(520)),
            Some(511..521)
        );
        assert_eq!(
            candidate_u64_mark_range(&index, 0, Bound::Excluded(512), Bound::Excluded(520)),
            Some(512..520)
        );
        assert_eq!(
            candidate_u64_mark_range(&index, 0, Bound::Unbounded, Bound::Unbounded),
            Some(0..8_192)
        );
    }
}

#[cfg(all(test, unix))]
mod raw_index_link_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn rejects_symlinked_primary_index_files() {
        let root =
            std::env::temp_dir().join(format!("rusthouse-raw-primary-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let real = root.join("real.idx");
        let link = root.join("link.idx");
        fs::write(&real, 42u64.to_le_bytes()).unwrap();
        symlink(&real, &link).unwrap();
        assert!(read_fixed_primary_index(&link, &[8]).is_err());
        assert_eq!(read_fixed_primary_index(&real, &[8]).unwrap().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod primary_index_size_boundary_tests {
    use super::*;

    #[test]
    fn raw_indexes_remain_bounded_and_exact() {
        let path =
            std::env::temp_dir().join(format!("rusthouse-raw-index-size-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        fs::write(&path, [42u8; 48]).unwrap();
        assert_eq!(read_raw(&path).unwrap(), vec![42; 48]);
        fs::write(&path, [43u8; 16]).unwrap();
        assert_eq!(read_raw(&path).unwrap(), vec![43; 16]);
        fs::remove_file(&path).unwrap();
    }
}

#[cfg(test)]
mod primary_index_short_read_tests {
    use super::*;

    #[test]
    fn rejects_short_indexes_that_remain_row_aligned() {
        let path = Path::new("synthetic.idx");
        assert!(ensure_exact_primary_index_length(path, 24, 16).is_err());
        assert!(ensure_exact_primary_index_length(path, 24, 24).is_ok());
    }
}

#[cfg(all(test, unix))]
mod raw_index_immutability_tests {
    use super::*;

    #[test]
    fn detects_same_sized_path_replacement_and_in_place_rewrite() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-index-immutability-{}-{}",
            std::process::id(),
            line!(),
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("primary.idx");
        let replacement = dir.join("replacement.idx");
        fs::write(&path, [3u8; 16]).unwrap();
        fs::write(&replacement, [9u8; 16]).unwrap();
        let before = fs::symlink_metadata(&path).unwrap();
        fs::rename(&replacement, &path).unwrap();
        let after = fs::symlink_metadata(&path).unwrap();
        assert_eq!(before.len(), after.len());
        assert!(!raw_index_unchanged(&before, &after));
        let original = fs::symlink_metadata(&path).unwrap();
        fs::write(&path, [8u8; 17]).unwrap();
        let changed = fs::symlink_metadata(&path).unwrap();
        assert!(!raw_index_unchanged(&original, &changed));
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod recycled_primary_index_decoder_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

    fn none_block(payload: &[u8]) -> Vec<u8> {
        let mut result = vec![0u8; 16];
        result.push(0x02);
        result.extend_from_slice(&(payload.len() as u32 + 9).to_le_bytes());
        result.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        result.extend_from_slice(payload);
        let checksum = u128::from(ch_cityhash102::cityhash128(&result[16..])).to_le_bytes();
        result[..16].copy_from_slice(&checksum);
        result
    }

    #[test]
    fn compressed_primary_index_reads_many_boundary_split_tuples() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-primary-index-recycled-{}-{}.cidx",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed),
        ));
        let raw = (0..1024u64)
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let mut compressed = Vec::new();
        for part in raw.chunks(7) {
            compressed.extend(none_block(part));
        }
        fs::write(&path, compressed).unwrap();
        let decoded = read_fixed_primary_index(&path, &[8]).unwrap();
        assert_eq!(decoded.len(), 1024);
        for index in [0, 1, 63, 511, 1023] {
            assert_eq!(
                u64::from_le_bytes(decoded.field(index, 0).unwrap().try_into().unwrap()),
                index as u64
            );
        }
        fs::remove_file(&path).unwrap();
    }
}

#[cfg(all(test, unix))]
mod compressed_index_identity_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEST: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn opened_compressed_index_identity_rejects_path_replacement() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-compressed-index-identity-{}-{}",
            std::process::id(), NEXT_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("primary.cidx");
        let encoded = {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&[0u8; 16]);
            bytes.push(0x02);
            bytes.extend_from_slice(&17u32.to_le_bytes());
            bytes.extend_from_slice(&8u32.to_le_bytes());
            bytes.extend_from_slice(&42u64.to_le_bytes());
            let checksum = u128::from(ch_cityhash102::cityhash128(&bytes[16..])).to_le_bytes();
            bytes[..16].copy_from_slice(&checksum);
            bytes
        };
        fs::write(&path, &encoded).unwrap();
        assert_eq!(read_fixed_primary_index(&path, &[8]).unwrap().len(), 1);
        let mut reader = CompressedBlockReader::open(&path).unwrap();
        let before = reader.opened_metadata().unwrap();
        assert!(raw_index_unchanged(&before, &fs::symlink_metadata(&path).unwrap()));
        fs::rename(&path, root.join("old.cidx")).unwrap();
        fs::write(&path, &encoded).unwrap();
        assert!(!raw_index_unchanged(&before, &fs::symlink_metadata(&path).unwrap()));
        assert!(reader.next_decoded().unwrap().is_ok());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod wide_integer_pruning_tests {
    use super::*;

    #[test]
    fn wide_unsigned_pruning_preserves_predecessor_and_extremes() {
        let keys = [1u128, 1u128 << 64, 1u128 << 100, u128::MAX];
        let index = FixedPrimaryIndex {
            data: keys.into_iter().flat_map(u128::to_le_bytes).collect(),
            field_offsets: vec![0, 16],
            row_width: 16,
        };
        assert_eq!(candidate_u128_mark_range(&index, 0,
            Bound::Included((1u128 << 64) + 1), Bound::Included(1u128 << 100)), Some(1..3));
        assert_eq!(candidate_u128_mark_ranges(&index, 0, &[
            (Bound::Included(1u128 << 100), Bound::Included(1u128 << 100)),
            (Bound::Included(1), Bound::Included(1)),
        ]), Some(vec![0..3]));
        assert_eq!(candidate_u128_mark_range(&index, 0,
            Bound::Included(u128::MAX), Bound::Included(u128::MAX)), Some(2..4));
        assert_eq!(candidate_u128_mark_range(&index, 1,
            Bound::Included(0), Bound::Included(1)), None);
    }

    #[test]
    fn wide_signed_pruning_compares_numeric_not_little_endian_order() {
        let keys = [i128::MIN, -1i128, 0, i128::MAX];
        let index = FixedPrimaryIndex {
            data: keys.into_iter().flat_map(i128::to_le_bytes).collect(),
            field_offsets: vec![0, 16],
            row_width: 16,
        };
        assert_eq!(candidate_i128_mark_range(&index, 0,
            Bound::Included(-1), Bound::Included(0)), Some(0..3));
        assert_eq!(candidate_i128_mark_ranges(&index, 0, &[
            (Bound::Included(i128::MIN), Bound::Included(i128::MIN)),
            (Bound::Included(i128::MAX), Bound::Included(i128::MAX)),
        ]), Some(vec![0..1, 2..4]));
        let invalid = FixedPrimaryIndex {
            data: [1i128, -1].into_iter().flat_map(i128::to_le_bytes).collect(),
            field_offsets: vec![0, 16], row_width: 16,
        };
        assert_eq!(candidate_i128_mark_range(&invalid, 0,
            Bound::Included(0), Bound::Included(1)), None);
    }
}

#[cfg(test)]
mod composite_mark_pruning_tests {
    use super::*;

    fn unsigned_index(pairs: &[(u64, u64)]) -> FixedPrimaryIndex {
        let data = pairs.iter().flat_map(|&(a, b)| [a.to_le_bytes(), b.to_le_bytes()].concat()).collect();
        FixedPrimaryIndex { data, field_offsets: vec![0, 8, 16], row_width: 16 }
    }

    #[test]
    fn lexicographic_order_handles_secondary_key_resets() {
        let index = unsigned_index(&[(1,0), (1,10), (1,20), (2,0), (2,5), (3,0)]);
        assert_eq!(candidate_u64_pair_mark_range(&index, 0, 1,
            Bound::Included((1,11)), Bound::Included((2,0))), Some(1..4));
        assert_eq!(candidate_u64_pair_mark_ranges(&index, 0, 1, &[
            (Bound::Included((1,0)), Bound::Included((1,0))),
            (Bound::Included((3,0)), Bound::Included((3,0))),
        ]), Some(vec![0..1, 4..6]));
        assert_eq!(candidate_u64_pair_mark_range(&index, 0, 2,
            Bound::Included((0,0)), Bound::Included((1,0))), None);
    }

    #[test]
    fn signed_tuple_order_preserves_negative_components() {
        let mut data = Vec::new();
        for (a,b) in [(-2i64, 99i64), (-2,100), (-1,-1), (-1,0), (0,-5)] {
            data.extend_from_slice(&a.to_le_bytes());
            data.extend_from_slice(&b.to_le_bytes());
        }
        let index = FixedPrimaryIndex { data, field_offsets: vec![0,8,16], row_width:16 };
        assert_eq!(candidate_i64_pair_mark_range(&index, 0, 1,
            Bound::Included((-1,-1)), Bound::Included((-1,0))), Some(1..4));
        assert_eq!(candidate_i64_pair_mark_ranges(&index, 0, 1, &[
            (Bound::Included((-2,99)), Bound::Included((-2,100))),
            (Bound::Included((0,-5)), Bound::Included((0,-5))),
        ]), Some(vec![0..2, 3..5]));
    }

    #[test]
    fn malformed_composite_order_fails_closed() {
        let invalid = unsigned_index(&[(1,10), (1,5)]);
        assert_eq!(candidate_u64_pair_mark_range(&invalid,0,1,
            Bound::Included((1,5)), Bound::Included((1,10))), None);
        assert_eq!(candidate_u64_pair_mark_ranges(&invalid,0,1,
            &[(Bound::Included((1,5)), Bound::Included((1,10)))]), None);
    }
}

#[cfg(test)]
mod narrow_integer_pruning_tests {
    use super::*;

    fn index(values: Vec<u8>, width: usize) -> FixedPrimaryIndex {
        FixedPrimaryIndex {
            data: values, field_offsets: vec![0, width], row_width: width,
        }
    }

    #[test]
    fn unsigned_ranges_preserve_width_and_type_boundaries() {
        let bytes = [1u8, 10, 200, 255].to_vec();
        let eight = index(bytes, 1);
        assert_eq!(candidate_narrow_unsigned_mark_range(&eight, 0,
            Bound::Included(11), Bound::Included(200)), Some(1..3));
        assert_eq!(candidate_narrow_unsigned_mark_range(&eight, 0,
            Bound::Included(256), Bound::Unbounded), Some(3..4));
        for (values, width) in [
            (vec![1u16, 256, 4096, u16::MAX].iter().flat_map(|v| v.to_le_bytes()).collect(), 2),
            (vec![1u32, 256, 4096, u32::MAX].iter().flat_map(|v| v.to_le_bytes()).collect(), 4),
        ] {
            let idx = index(values, width);
            assert_eq!(candidate_narrow_unsigned_mark_range(&idx, 0,
                Bound::Included(257), Bound::Included(4096)), Some(1..3));
            assert_eq!(candidate_narrow_unsigned_mark_ranges(&idx, 0, &[
                (Bound::Included(1), Bound::Included(1)),
                (Bound::Included(4096), Bound::Included(4096)),
            ]), Some(vec![0..3]));
        }
    }

    #[test]
    fn signed_ranges_use_signed_numeric_order() {
        let eight = index(vec![i8::MIN as u8, 255, 0, i8::MAX as u8], 1);
        assert_eq!(candidate_narrow_signed_mark_range(&eight, 0,
            Bound::Included(-1), Bound::Included(0)), Some(0..3));
        for (values, width) in [
            (vec![i16::MIN, -10, 0, i16::MAX].iter().flat_map(|v| v.to_le_bytes()).collect(), 2),
            (vec![i32::MIN, -10, 0, i32::MAX].iter().flat_map(|v| v.to_le_bytes()).collect(), 4),
        ] {
            let idx = index(values, width);
            assert_eq!(candidate_narrow_signed_mark_range(&idx, 0,
                Bound::Included(-9), Bound::Included(0)), Some(1..3));
        }
    }

    #[test]
    fn invalid_widths_and_unsorted_keys_fail_closed() {
        let invalid = index(vec![4,3,2], 1);
        assert_eq!(candidate_narrow_unsigned_mark_range(&invalid, 0,
            Bound::Included(0), Bound::Included(4)), None);
        let wrong_width = index(9u64.to_le_bytes().to_vec(), 8);
        assert_eq!(candidate_narrow_unsigned_mark_range(&wrong_width, 0,
            Bound::Included(0), Bound::Included(9)), None);
        assert_eq!(candidate_narrow_signed_mark_range(&wrong_width, 0,
            Bound::Included(0), Bound::Included(9)), None);
    }
}
