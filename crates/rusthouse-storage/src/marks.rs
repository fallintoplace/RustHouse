use crate::{CompressedBlockReader, PartError};
use std::fs;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

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

pub fn read_wide_marks(path: impl AsRef<Path>) -> Result<WideMarks, PartError> {
    let path = path.as_ref();
    let (format, compressed) = match path.extension().and_then(|extension| extension.to_str()) {
        Some("mrk") => (WideMarkFormat::Fixed, false),
        Some("mrk2") => (WideMarkFormat::Adaptive, false),
        Some("cmrk") => (WideMarkFormat::Fixed, true),
        Some("cmrk2") => (WideMarkFormat::Adaptive, true),
        _ => return Err(PartError::UnsupportedMarksFormat(path.to_path_buf())),
    };
    let marks = if compressed {
        read_compressed_wide_marks(CompressedBlockReader::open(path)?, path, format)?
    } else {
        let content = fs::read(path).map_err(|source| PartError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        parse_wide_marks(&content, format, path)?
    };
    Ok(WideMarks {
        path: path.to_path_buf(),
        format,
        marks,
    })
}

fn parse_wide_marks(
    content: &[u8],
    format: WideMarkFormat,
    path: &Path,
) -> Result<Vec<WideMark>, PartError> {
    if content.is_empty() {
        return Err(PartError::EmptyMarksFile(path.to_path_buf()));
    }

    let record_size = format.record_size();
    if content.len() % record_size != 0 {
        return Err(PartError::InvalidMarksLength {
            path: path.to_path_buf(),
            size: content.len(),
            record_size,
        });
    }

    Ok(content
        .chunks_exact(record_size)
        .map(|record| parse_mark_record(record, format))
        .collect())
}

fn read_compressed_wide_marks<R: Read + Seek>(
    mut reader: CompressedBlockReader<R>,
    path: &Path,
    format: WideMarkFormat,
) -> Result<Vec<WideMark>, PartError> {
    let record_size = format.record_size();
    let mut marks = Vec::new();
    let mut fragment = [0u8; 24];
    let mut fragment_len = 0;

    while let Some(block) = reader.next_decoded() {
        let data = block?.data;
        let mut bytes = data.as_slice();

        if fragment_len > 0 {
            let take = (record_size - fragment_len).min(bytes.len());
            fragment[fragment_len..fragment_len + take].copy_from_slice(&bytes[..take]);
            fragment_len += take;
            bytes = &bytes[take..];
            if fragment_len < record_size {
                continue;
            }
            marks.push(parse_mark_record(&fragment[..record_size], format));
        }

        let mut chunks = bytes.chunks_exact(record_size);
        marks.extend(chunks.by_ref().map(|record| parse_mark_record(record, format)));
        let remainder = chunks.remainder();
        fragment[..remainder.len()].copy_from_slice(remainder);
        fragment_len = remainder.len();
    }

    let size = marks.len() * record_size + fragment_len;
    if size == 0 {
        return Err(PartError::EmptyMarksFile(path.to_path_buf()));
    }
    if fragment_len != 0 {
        return Err(PartError::InvalidMarksLength {
            path: path.to_path_buf(),
            size,
            record_size,
        });
    }
    Ok(marks)
}

fn parse_mark_record(record: &[u8], format: WideMarkFormat) -> WideMark {
    WideMark {
        compressed_offset: read_u64(&record[..8]),
        decompressed_offset: read_u64(&record[8..16]),
        rows: (format == WideMarkFormat::Adaptive).then(|| read_u64(&record[16..24])),
    }
}

fn read_u64(bytes: &[u8]) -> u64 {
    let mut value = [0; 8];
    value.copy_from_slice(bytes);
    u64::from_le_bytes(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ch_cityhash102::cityhash128;
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

    fn read_compressed(
        bytes: Vec<u8>,
        format: WideMarkFormat,
    ) -> Result<Vec<WideMark>, PartError> {
        let path = Path::new("wide.cmrk2");
        let reader = CompressedBlockReader::new(Cursor::new(bytes), path)?;
        read_compressed_wide_marks(reader, path, format)
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
        for path in [
            "marks.mrk3",
            "marks.mrk4",
            "marks.cmrk3",
            "marks.cmrk4",
        ] {
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
}
