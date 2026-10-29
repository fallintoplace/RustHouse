mod streams;

use self::streams::*;

use crate::scan_layout::ScanLayout;
use crate::{
    CompressedBlockReader, PartError, WideMarkFormat, WideMarks, inspect_part, read_wide_marks,
    validate_wide_marks,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartVerification {
    pub streams: usize,
    pub marks: usize,
    pub blocks: u64,
    pub decompressed_bytes: u64,
}

pub fn verify_wide_part(path: impl AsRef<Path>) -> Result<PartVerification, PartError> {
    let path = path.as_ref();
    let metadata = inspect_part(path)?;
    let (data_files, mut mark_files) = collect_wide_file_pairs(path)?;

    validate_stream_pairs(path, metadata.rows, &data_files, &mark_files)?;

    let mut report = PartVerification {
        streams: data_files.len(),
        marks: 0,
        blocks: 0,
        decompressed_bytes: 0,
    };
    let mut layout = ScanLayout::default();
    for data in data_files {
        let marks_path = mark_files.remove(&data).unwrap();
        let marks = read_wide_marks(marks_path)?;
        validate_adaptive_stream(&data, &marks, metadata.rows, &mut layout)?;
        validate_wide_marks(&marks, CompressedBlockReader::open(&data)?)?;

        let decoded_size = account_decoded_blocks(&data, &mut report)?;
        if let Some(expected) = expected_fixed_stream_bytes(&data, &metadata)? {
            if decoded_size != expected {
                return Err(invalid(
                    &data,
                    "fixed-width stream byte count differs from count.txt",
                ));
            }
        }
        report.marks = report
            .marks
            .checked_add(marks.marks.len())
            .ok_or_else(|| invalid(&data, "mark count overflows"))?;
    }
    Ok(report)
}

/// Validate only streams with an unambiguous fixed-width on-disk representation.
/// Variable-length and nested types still rely on their dedicated readers.
fn fixed_stream_width(kind: &str) -> Option<u64> {
    let kind = kind
        .strip_prefix("Nullable(")
        .and_then(|base| base.strip_suffix(')'))
        .unwrap_or(kind);
    match kind {
        "Bool" | "UInt8" | "Int8" => Some(1),
        "Date" | "UInt16" | "Int16" | "BFloat16" => Some(2),
        "DateTime" | "UInt32" | "Int32" | "Float32" | "IPv4" => Some(4),
        "UInt64" | "Int64" | "Float64" => Some(8),
        "UInt128" | "Int128" | "UUID" | "IPv6" => Some(16),
        "UInt256" | "Int256" => Some(32),
        _ => kind
            .strip_prefix("FixedString(")
            .and_then(|width| width.strip_suffix(')'))
            .and_then(|width| width.parse::<u64>().ok())
            .filter(|width| *width > 0),
    }
}

fn expected_fixed_stream_bytes(
    data: &Path,
    metadata: &crate::PartMetadata,
) -> Result<Option<u64>, PartError> {
    let Some(name) = data
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".bin"))
    else {
        return Ok(None);
    };
    let width = if let Some(name) = name.strip_suffix(".null") {
        if metadata
            .columns
            .iter()
            .any(|column| column.name == name && column.data_type.starts_with("Nullable("))
        {
            Some(1)
        } else {
            None
        }
    } else {
        metadata
            .columns
            .iter()
            .find(|column| column.name == name)
            .and_then(|column| fixed_stream_width(&column.data_type))
    };
    width
        .map(|width| {
            metadata
                .rows
                .checked_mul(width)
                .ok_or_else(|| invalid(data, "fixed-width stream byte count overflows"))
        })
        .transpose()
}

fn invalid(path: &Path, reason: &'static str) -> PartError {
    PartError::InvalidPart {
        path: path.to_path_buf(),
        reason,
    }
}

fn io_error(path: &Path, source: io::Error) -> PartError {
    PartError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ch_cityhash102::cityhash128;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    fn fixture() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-verify-part-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("count.txt"), "2\n").unwrap();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n1 columns:\n`id` UInt64\n",
        )
        .unwrap();
        let bytes: Vec<_> = [10u64, 20].into_iter().flat_map(u64::to_le_bytes).collect();
        let mut encoded = vec![0x02];
        encoded.extend_from_slice(&(9 + bytes.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&bytes);
        let mut block = u128::from(cityhash128(&encoded)).to_le_bytes().to_vec();
        block.extend_from_slice(&encoded);
        fs::write(dir.join("id.bin"), block).unwrap();
        let mut mark = Vec::new();
        mark.extend_from_slice(&0u64.to_le_bytes());
        mark.extend_from_slice(&0u64.to_le_bytes());
        mark.extend_from_slice(&2u64.to_le_bytes());
        fs::write(dir.join("id.mrk2"), mark).unwrap();
        dir
    }

    #[test]
    fn verifies_stream_blocks_and_adaptive_mark_offsets() {
        let dir = fixture();
        assert_eq!(
            verify_wide_part(&dir).unwrap(),
            PartVerification {
                streams: 1,
                marks: 1,
                blocks: 1,
                decompressed_bytes: 16,
            }
        );
        fs::remove_dir_all(dir).unwrap();
    }

    fn checked_plain_block(data: &[u8]) -> Vec<u8> {
        let mut encoded = vec![0x02];
        encoded.extend_from_slice(&(9 + data.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&(data.len() as u32).to_le_bytes());
        encoded.extend_from_slice(data);
        let mut block = u128::from(cityhash128(&encoded)).to_le_bytes().to_vec();
        block.extend_from_slice(&encoded);
        block
    }

    #[test]
    fn checks_fixed_width_stream_sizes_against_row_counts() {
        let dir = fixture();
        for values in [vec![10u64], vec![10, 20, 30]] {
            let bytes: Vec<_> = values.into_iter().flat_map(u64::to_le_bytes).collect();
            fs::write(dir.join("id.bin"), checked_plain_block(&bytes)).unwrap();
            assert!(matches!(
                verify_wide_part(&dir),
                Err(PartError::InvalidPart {
                    reason: "fixed-width stream byte count differs from count.txt",
                    ..
                })
            ));
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn checks_nullable_null_maps_and_fixed_string_widths() {
        let dir = fixture();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n1 columns:\n`id` Nullable(UInt64)\n",
        )
        .unwrap();
        fs::write(dir.join("id.null.bin"), checked_plain_block(&[0, 1])).unwrap();
        fs::copy(dir.join("id.mrk2"), dir.join("id.null.mrk2")).unwrap();
        assert!(verify_wide_part(&dir).is_ok());
        fs::write(dir.join("id.null.bin"), checked_plain_block(&[0])).unwrap();
        assert!(matches!(
            verify_wide_part(&dir),
            Err(PartError::InvalidPart {
                reason: "fixed-width stream byte count differs from count.txt",
                ..
            })
        ));
        fs::remove_file(dir.join("id.null.bin")).unwrap();
        fs::remove_file(dir.join("id.null.mrk2")).unwrap();

        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n1 columns:\n`id` FixedString(6)\n",
        )
        .unwrap();
        fs::write(dir.join("id.bin"), checked_plain_block(b"abcdefghijkl")).unwrap();
        assert!(verify_wide_part(&dir).is_ok());
        fs::write(dir.join("id.bin"), checked_plain_block(b"abcdefghijk")).unwrap();
        assert!(matches!(
            verify_wide_part(&dir),
            Err(PartError::InvalidPart {
                reason: "fixed-width stream byte count differs from count.txt",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn variable_width_streams_are_not_mistaken_for_fixed_width() {
        let dir = fixture();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n1 columns:\n`id` String\n",
        )
        .unwrap();
        assert!(verify_wide_part(&dir).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_corrupt_block_and_invalid_mark_offsets() {
        let dir = fixture();
        let path = dir.join("id.bin");
        let mut block = fs::read(&path).unwrap();
        block[25] ^= 1;
        fs::write(&path, block).unwrap();
        assert!(matches!(
            verify_wide_part(&dir),
            Err(PartError::InvalidCompressedBlock {
                reason: "checksum mismatch",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();

        let dir = fixture();
        let path = dir.join("id.mrk2");
        let mut marks = fs::read(&path).unwrap();
        marks[..8].copy_from_slice(&1u64.to_le_bytes());
        fs::write(&path, marks).unwrap();
        assert!(matches!(
            verify_wide_part(&dir),
            Err(PartError::InvalidMarkOffset { .. })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_mismatched_rows_and_missing_or_duplicate_pairs() {
        let dir = fixture();
        fs::write(dir.join("count.txt"), "3\n").unwrap();
        assert!(matches!(
            verify_wide_part(&dir),
            Err(PartError::InvalidPart {
                reason: "mark row counts do not match count.txt",
                ..
            })
        ));
        fs::write(dir.join("count.txt"), "2\n").unwrap();

        fs::write(dir.join("id.cmrk2"), [1u8, 2]).unwrap();
        assert!(matches!(
            verify_wide_part(&dir),
            Err(PartError::InvalidPart {
                reason: "multiple mark files for one stream",
                ..
            })
        ));
        fs::remove_file(dir.join("id.cmrk2")).unwrap();

        fs::remove_file(dir.join("id.mrk2")).unwrap();
        assert!(matches!(
            verify_wide_part(&dir),
            Err(PartError::InvalidPart {
                reason: "data stream has no matching marks",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn checks_shared_granule_boundaries_across_streams() {
        let dir = fixture();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n2 columns:\n`id` UInt64\n`score` UInt64\n",
        )
        .unwrap();
        fs::copy(dir.join("id.bin"), dir.join("score.bin")).unwrap();
        fs::copy(dir.join("id.mrk2"), dir.join("score.mrk2")).unwrap();
        assert_eq!(
            verify_wide_part(&dir).unwrap(),
            PartVerification {
                streams: 2,
                marks: 2,
                blocks: 2,
                decompressed_bytes: 32,
            }
        );

        // The total row count still matches count.txt, but the score column
        // splits the two rows into different granules.
        let mut marks = Vec::new();
        for offset in [0u64, 8] {
            marks.extend_from_slice(&0u64.to_le_bytes());
            marks.extend_from_slice(&offset.to_le_bytes());
            marks.extend_from_slice(&1u64.to_le_bytes());
        }
        fs::write(dir.join("score.mrk2"), marks).unwrap();
        assert!(matches!(
            verify_wide_part(&dir),
            Err(PartError::InvalidPart {
                reason: "adaptive streams have mismatched granule boundaries",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_part_without_streams_is_valid() {
        let dir = fixture();
        fs::write(dir.join("count.txt"), "0\n").unwrap();
        fs::remove_file(dir.join("id.bin")).unwrap();
        fs::remove_file(dir.join("id.mrk2")).unwrap();
        assert_eq!(
            verify_wide_part(&dir).unwrap(),
            PartVerification {
                streams: 0,
                marks: 0,
                blocks: 0,
                decompressed_bytes: 0,
            }
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
