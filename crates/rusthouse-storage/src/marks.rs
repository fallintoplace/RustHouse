mod read_helpers;
use self::read_helpers::*;

mod parsing;

use self::parsing::*;

use crate::{CompressedBlockReader, PartError};
use std::fs::File;
use std::io::{self, Read, Seek};

use std::path::{Path, PathBuf};

const MAX_MARKS_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WideMarkFormat {
    Fixed,
    Adaptive,
}

impl WideMarkFormat {
    fn record_size(self) -> usize {
        match self {
            Self::Fixed => 16,
            Self::Adaptive => 24,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WideMark {
    pub compressed_offset: u64,
    pub decompressed_offset: u64,
    pub rows: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WideMarks {
    pub path: PathBuf,
    pub format: WideMarkFormat,
    pub marks: Vec<WideMark>,
}

fn marks_format(path: &Path) -> Result<(WideMarkFormat, bool), PartError> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("mrk") => Ok((WideMarkFormat::Fixed, false)),
        Some("mrk2") => Ok((WideMarkFormat::Adaptive, false)),
        Some("cmrk") => Ok((WideMarkFormat::Fixed, true)),
        Some("cmrk2") => Ok((WideMarkFormat::Adaptive, true)),
        _ => Err(PartError::UnsupportedMarksFormat(path.to_path_buf())),
    }
}

pub fn read_wide_marks(path: impl AsRef<Path>) -> Result<WideMarks, PartError> {
    let path = path.as_ref();
    let (format, compressed) = marks_format(path)?;
    let marks = if compressed {
        read_compressed_wide_marks(CompressedBlockReader::open(path)?, path, format)?
    } else {
        read_plain_wide_marks(path, format)?
    };
    Ok(WideMarks {
        path: path.to_path_buf(),
        format,
        marks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ch_cityhash102::cityhash128;
    use std::fs;
    use std::io::Cursor;

    fn checked_block(method: u8, data: &[u8]) -> Vec<u8> {
        let payload = match method {
            0x02 => data.to_vec(),
            0x82 => lz4_flex::block::compress(data),
            0x90 => zstd::bulk::compress(data, 1).unwrap(),
            _ => unreachable!(),
        };
        let mut encoded = vec![method];
        encoded.extend_from_slice(&(9u32 + payload.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&(data.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&payload);
        let mut bytes = u128::from(cityhash128(&encoded)).to_le_bytes().to_vec();
        bytes.extend_from_slice(&encoded);
        bytes
    }

    fn read_compressed(bytes: Vec<u8>, format: WideMarkFormat) -> Result<Vec<WideMark>, PartError> {
        let path = Path::new("wide.cmrk2");
        let reader = CompressedBlockReader::new(Cursor::new(bytes), path)?;
        read_compressed_wide_marks(reader, path, format)
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_uncompressed_marks() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!(
            "rusthouse-plain-marks-symlink-{}",
            std::process::id()
        ));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("actual.mrk2"), [0u8; 24]).unwrap();
        symlink("actual.mrk2", dir.join("link.mrk2")).unwrap();
        assert!(matches!(
            read_wide_marks(dir.join("link.mrk2")),
            Err(PartError::Io { source, .. }) if source.kind() == io::ErrorKind::InvalidInput
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_compressed_wide_marks_across_mixed_block_boundaries() {
        let mut records = Vec::new();
        for (compressed, decompressed, rows) in [(0u64, 0u64, 32u64), (90, 8, 10)] {
            records.extend_from_slice(&compressed.to_le_bytes());
            records.extend_from_slice(&decompressed.to_le_bytes());
            records.extend_from_slice(&rows.to_le_bytes());
        }
        let mut bytes = checked_block(0x82, &records[..11]);
        bytes.extend(checked_block(0x90, &records[11..29]));
        bytes.extend(checked_block(0x02, &records[29..]));

        let marks = read_compressed(bytes, WideMarkFormat::Adaptive).unwrap();
        assert_eq!(
            marks,
            vec![
                WideMark {
                    compressed_offset: 0,
                    decompressed_offset: 0,
                    rows: Some(32),
                },
                WideMark {
                    compressed_offset: 90,
                    decompressed_offset: 8,
                    rows: Some(10),
                },
            ]
        );
    }

    #[test]
    fn dispatches_both_compressed_wide_mark_formats() {
        for (extension, format) in [
            ("cmrk", WideMarkFormat::Fixed),
            ("cmrk2", WideMarkFormat::Adaptive),
        ] {
            let mut record = Vec::new();
            record.extend_from_slice(&100u64.to_le_bytes());
            record.extend_from_slice(&8u64.to_le_bytes());
            if format == WideMarkFormat::Adaptive {
                record.extend_from_slice(&21u64.to_le_bytes());
            }
            let path = std::env::temp_dir().join(format!(
                "rusthouse-compressed-marks-{}.{}",
                std::process::id(),
                extension
            ));
            fs::write(&path, checked_block(0x82, &record)).unwrap();
            let result = read_wide_marks(&path);
            fs::remove_file(&path).unwrap();

            let marks = result.unwrap();
            assert_eq!(marks.path, path);
            assert_eq!(marks.format, format);
            assert_eq!(
                marks.marks,
                vec![WideMark {
                    compressed_offset: 100,
                    decompressed_offset: 8,
                    rows: (format == WideMarkFormat::Adaptive).then_some(21),
                }]
            );
        }
    }

    #[test]
    fn rejects_corrupt_compressed_marks_even_after_a_complete_record() {
        let mut bytes = checked_block(0x82, &[0u8; 24]);
        let mut second = checked_block(0x90, &[0u8; 24]);
        second[0] ^= 1;
        bytes.extend(second);

        assert!(matches!(
            read_compressed(bytes, WideMarkFormat::Adaptive),
            Err(PartError::InvalidCompressedBlock {
                reason: "checksum mismatch",
                ..
            })
        ));
    }

    #[test]
    fn rejects_truncated_compressed_mark_records() {
        for length in [1usize, 23, 25] {
            let bytes = checked_block(0x82, &vec![0u8; length]);
            assert!(matches!(
                read_compressed(bytes, WideMarkFormat::Adaptive),
                Err(PartError::InvalidMarksLength {
                    size,
                    record_size: 24,
                    ..
                }) if size == length
            ));
        }
    }

    #[test]
    fn rejects_empty_and_trailing_corrupt_compressed_marks() {
        assert!(matches!(
            read_compressed(Vec::new(), WideMarkFormat::Fixed),
            Err(PartError::EmptyMarksFile(_))
        ));

        let mut bytes = checked_block(0x02, &[0u8; 16]);
        bytes.push(0);
        assert!(matches!(
            read_compressed(bytes, WideMarkFormat::Fixed),
            Err(PartError::InvalidCompressedBlock { .. })
        ));
    }

    #[test]
    fn reads_adaptive_wide_marks() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/marks/wide.mrk2");
        let file = read_wide_marks(&path).unwrap();
        assert_eq!(file.path, path);
        assert_eq!(file.format, WideMarkFormat::Adaptive);
        assert_eq!(
            file.marks,
            vec![
                WideMark {
                    compressed_offset: 0,
                    decompressed_offset: 0,
                    rows: Some(32),
                },
                WideMark {
                    compressed_offset: 90,
                    decompressed_offset: 8,
                    rows: Some(10),
                },
            ]
        );
    }

    #[test]
    fn reads_fixed_wide_marks() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/marks/fixed.mrk");
        let file = read_wide_marks(path).unwrap();
        assert_eq!(file.format, WideMarkFormat::Fixed);
        assert_eq!(
            file.marks,
            vec![
                WideMark {
                    compressed_offset: 0,
                    decompressed_offset: 0,
                    rows: None,
                },
                WideMark {
                    compressed_offset: 24,
                    decompressed_offset: 64,
                    rows: None,
                },
            ]
        );
    }

    #[test]
    fn parses_little_endian_offsets_and_zero_row_sentinel() {
        let mut content = Vec::new();
        content.extend_from_slice(&0x0102030405060708u64.to_le_bytes());
        content.extend_from_slice(&17u64.to_le_bytes());
        content.extend_from_slice(&0u64.to_le_bytes());
        let marks =
            parse_wide_marks(&content, WideMarkFormat::Adaptive, Path::new("test.mrk2")).unwrap();
        assert_eq!(marks[0].compressed_offset, 0x0102030405060708);
        assert_eq!(marks[0].decompressed_offset, 17);
        assert_eq!(marks[0].rows, Some(0));
    }

    #[test]
    fn rejects_empty_marks() {
        for format in [WideMarkFormat::Fixed, WideMarkFormat::Adaptive] {
            assert!(matches!(
                parse_wide_marks(&[], format, Path::new("empty")),
                Err(PartError::EmptyMarksFile(_))
            ));
        }
    }

    #[test]
    fn rejects_truncated_records() {
        for (format, length) in [
            (WideMarkFormat::Fixed, 1),
            (WideMarkFormat::Fixed, 17),
            (WideMarkFormat::Adaptive, 23),
            (WideMarkFormat::Adaptive, 25),
        ] {
            let content = vec![0; length];
            assert!(matches!(
                parse_wide_marks(&content, format, Path::new("broken")),
                Err(PartError::InvalidMarksLength { size, .. }) if size == length
            ));
        }
    }

    #[test]
    fn refuses_compact_mark_formats() {
        for path in ["marks.mrk3", "marks.mrk4", "marks.cmrk3", "marks.cmrk4"] {
            assert!(
                matches!(
                    read_wide_marks(path),
                    Err(PartError::UnsupportedMarksFormat(_))
                ),
                "{path}"
            );
        }
    }

    #[test]
    fn reports_missing_mark_file() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/marks/missing.mrk2");
        assert!(matches!(
            read_wide_marks(&path),
            Err(PartError::Io { path: missing, .. }) if missing == path
        ));
    }

    #[test]
    fn rejects_oversized_sparse_raw_marks() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-oversized-marks-{}.mrk2",
            std::process::id()
        ));
        let file = File::create(&path).unwrap();
        file.set_len(MAX_MARKS_BYTES as u64 + 1).unwrap();
        assert!(matches!(
            read_wide_marks(&path),
            Err(PartError::Io { source, .. })
                if source.kind() == io::ErrorKind::InvalidData
        ));
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn reads_thousands_of_raw_marks_from_one_file() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-many-raw-marks-{}.mrk",
            std::process::id()
        ));
        let mut bytes = Vec::new();
        for mark in 0..4_096u64 {
            bytes.extend_from_slice(&(mark * 16).to_le_bytes());
            bytes.extend_from_slice(&0u64.to_le_bytes());
        }
        fs::write(&path, bytes).unwrap();
        let result = read_wide_marks(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(result.marks.len(), 4_096);
        assert_eq!(result.marks[4_095].compressed_offset, 4_095 * 16);
    }
    #[test]
    fn reconstructs_adaptive_marks_at_every_block_split() {
        let mut record = Vec::new();
        record.extend_from_slice(&7u64.to_le_bytes());
        record.extend_from_slice(&9u64.to_le_bytes());
        record.extend_from_slice(&12u64.to_le_bytes());
        let expected = vec![WideMark {
            compressed_offset: 7,
            decompressed_offset: 9,
            rows: Some(12),
        }];
        for split in 1..record.len() {
            let mut bytes = checked_block(0x02, &record[..split]);
            bytes.extend(checked_block(0x02, &record[split..]));
            assert_eq!(
                read_compressed(bytes, WideMarkFormat::Adaptive).unwrap(),
                expected,
                "split {split}"
            );
        }
    }
}

#[cfg(test)]
mod raw_mark_identity_tests {
    use super::*;

    #[test]
    fn raw_mark_reader_accepts_regular_files_only() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-raw-mark-identity-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("one.mrk");
        std::fs::write(&path, [0u8; 16]).unwrap();
        assert_eq!(read_wide_marks(&path).unwrap().marks.len(), 1);
        assert!(read_wide_marks(dir.join("missing.mrk")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod raw_marks_size_stability_tests {
    use super::*;
    use std::fs;

    #[test]
    fn rejects_truncated_raw_mark_records() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-raw-mark-size-{}.mrk2",
            std::process::id()
        ));
        fs::write(&path, [0u8; 23]).unwrap();
        assert!(read_wide_marks(&path).is_err());
        fs::write(&path, [0u8; 24]).unwrap();
        assert_eq!(read_wide_marks(&path).unwrap().marks.len(), 1);
        fs::remove_file(path).unwrap();
    }
}

#[cfg(all(test, unix))]
mod immutable_raw_marks_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn rejects_equal_length_replacement_and_ctime_changes() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-mark-identity-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        std::fs::create_dir_all(&root).unwrap();
        let original = root.join("first.mrk2");
        let replacement = root.join("second.mrk2");
        std::fs::write(&original, [0u8; 24]).unwrap();
        std::fs::write(&replacement, [1u8; 24]).unwrap();
        let before = std::fs::metadata(&original).unwrap();
        assert!(marks_file_unchanged(&before, &before));
        assert!(!marks_file_unchanged(
            &before,
            &std::fs::metadata(&replacement).unwrap(),
        ));
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut permissions = before.permissions();
        permissions.set_mode(permissions.mode() ^ 0o100);
        std::fs::set_permissions(&original, permissions).unwrap();
        let altered = std::fs::metadata(&original).unwrap();
        assert_eq!(altered.len(), before.len());
        assert_eq!(altered.modified().unwrap(), before.modified().unwrap());
        assert!(!marks_file_unchanged(&before, &altered));
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod bounded_plain_mark_reader_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn records_across_24_kib_reads_preserve_fixed_and_adaptive_offsets() {
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        for format in [WideMarkFormat::Fixed, WideMarkFormat::Adaptive] {
            let extension = if format == WideMarkFormat::Fixed {
                "mrk"
            } else {
                "mrk2"
            };
            let path = std::env::temp_dir().join(format!(
                "rusthouse-mark-batch-{}-{serial}.{extension}",
                std::process::id()
            ));
            let mut encoded = Vec::new();
            for index in 0..2053u64 {
                encoded.extend_from_slice(&(index * 128).to_le_bytes());
                encoded.extend_from_slice(&(index * 7).to_le_bytes());
                if format == WideMarkFormat::Adaptive {
                    encoded.extend_from_slice(&(index % 7 + 1).to_le_bytes());
                }
            }
            std::fs::write(&path, &encoded).unwrap();
            let parsed = read_wide_marks(&path).unwrap();
            std::fs::remove_file(&path).unwrap();
            assert_eq!(parsed.marks.len(), 2053);
            for index in [0usize, 1023, 1024, 2047, 2052] {
                let mark = &parsed.marks[index];
                assert_eq!(mark.compressed_offset, index as u64 * 128);
                assert_eq!(mark.decompressed_offset, index as u64 * 7);
                assert_eq!(
                    mark.rows,
                    (format == WideMarkFormat::Adaptive).then_some(index as u64 % 7 + 1)
                );
            }
        }
    }
}

#[cfg(test)]
mod plain_marks_open_stability_tests {
    use super::*;

    #[test]
    fn same_length_rewritten_marks_fail_the_open_metadata_guard() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-marks-open-stability-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("marks.mrk2");
        std::fs::write(&path, [0u8; 24]).unwrap();
        let before = std::fs::symlink_metadata(&path).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        assert!(marks_file_unchanged(&before, &file.metadata().unwrap()));
        file.set_modified(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        assert!(!marks_file_unchanged(&before, &file.metadata().unwrap()));
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod plain_marks_size_preflight_tests {
    use super::*;

    #[test]
    fn rejects_invalid_record_lengths_without_decoding_entire_file() {
        for (extension, record_size) in [("mrk", 16usize), ("mrk2", 24usize)] {
            let path = std::env::temp_dir().join(format!(
                "rusthouse-plain-marks-preflight-{}-{:?}.{}",
                std::process::id(),
                std::thread::current().id(),
                extension
            ));
            let file = File::create(&path).unwrap();
            file.set_len(1_048_577).unwrap();
            drop(file);
            assert!(matches!(
                read_wide_marks(&path),
                Err(PartError::InvalidMarksLength { size: 1_048_577, record_size: width, .. }) if width == record_size
            ));
            std::fs::remove_file(path).unwrap();
        }
    }
}

#[cfg(test)]
mod recycled_compressed_marks_tests {
    use super::*;
    use ch_cityhash102::cityhash128;
    use std::io::Cursor;

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut encoded = vec![0u8; 16];
        encoded.push(0x02);
        encoded.extend_from_slice(&(payload.len() as u32 + 9).to_le_bytes());
        encoded.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        encoded.extend_from_slice(payload);
        let hash = u128::from(cityhash128(&encoded[16..])).to_le_bytes();
        encoded[..16].copy_from_slice(&hash);
        encoded
    }

    #[test]
    fn recycled_frames_preserve_marks_across_multiple_incomplete_records() {
        let expected = (0..4u64)
            .map(|index| WideMark {
                compressed_offset: index * 128,
                decompressed_offset: 0,
                rows: Some(index + 1),
            })
            .collect::<Vec<_>>();
        let mut plain = Vec::new();
        for mark in &expected {
            plain.extend_from_slice(&mark.compressed_offset.to_le_bytes());
            plain.extend_from_slice(&mark.decompressed_offset.to_le_bytes());
            plain.extend_from_slice(&mark.rows.unwrap().to_le_bytes());
        }
        for chunk_width in [1, 3, 7, 23, 24, 25, 47, 64] {
            let mut encoded = Vec::new();
            for bytes in plain.chunks(chunk_width) {
                encoded.extend_from_slice(&frame(bytes));
            }
            let reader = CompressedBlockReader::new(Cursor::new(encoded), "marks.cmrk2").unwrap();
            let actual = read_compressed_wide_marks(
                reader,
                Path::new("marks.cmrk2"),
                WideMarkFormat::Adaptive,
            )
            .unwrap();
            assert_eq!(actual, expected, "chunk width {chunk_width}");
        }
    }
}
