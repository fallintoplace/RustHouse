mod range_keys;

use self::range_keys::*;

use crate::{CompressedBlockReader, PartError};
use std::fs::{self, File};
use std::io::Read;
use std::ops::{Bound, Range};
use std::path::Path;

const MAX_INDEX_BYTES: usize = 256 * 1024 * 1024;
const MAX_INDEX_FIELDS: usize = 4096;

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

fn read_raw(path: &Path) -> Result<Vec<u8>, PartError> {
    let original = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !original.file_type().is_file() {
        return Err(invalid(path, "primary index must be a regular file"));
    }
    let file = File::open(path).map_err(|source| io_error(path, source))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let opened = file.metadata().map_err(|source| io_error(path, source))?;
        if !opened.is_file() || original.dev() != opened.dev() || original.ino() != opened.ino() {
            return Err(invalid(path, "primary index replaced during open"));
        }
    }
    let size = file
        .metadata()
        .map_err(|source| io_error(path, source))?
        .len();
    if size > MAX_INDEX_BYTES as u64 {
        return Err(invalid(path, "index exceeds the 256 MiB limit"));
    }
    // The checked file size is known. Avoid repeated growth for large raw indexes.
    let mut data = Vec::with_capacity(size as usize);
    file.take(MAX_INDEX_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|source| io_error(path, source))?;
    if data.len() > MAX_INDEX_BYTES {
        return Err(invalid(path, "index exceeds the 256 MiB limit"));
    }
    Ok(data)
}

fn read_compressed(path: &Path) -> Result<Vec<u8>, PartError> {
    let mut blocks = CompressedBlockReader::open(path)?;
    let mut data = Vec::new();
    while let Some(block) = blocks.next_decoded() {
        let block = block?;
        if block.data.len() > MAX_INDEX_BYTES - data.len() {
            return Err(invalid(path, "index exceeds the 256 MiB limit"));
        }
        data.extend_from_slice(&block.data);
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
        let root = std::env::temp_dir().join(format!(
            "rusthouse-raw-primary-link-{}",
            std::process::id()
        ));
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
