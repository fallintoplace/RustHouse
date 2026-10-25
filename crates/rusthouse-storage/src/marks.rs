use crate::PartError;
use std::fs;
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
    let format = match path.extension().and_then(|extension| extension.to_str()) {
        Some("mrk") => WideMarkFormat::Fixed,
        Some("mrk2") => WideMarkFormat::Adaptive,
        _ => return Err(PartError::UnsupportedMarksFormat(path.to_path_buf())),
    };
    let content = fs::read(path).map_err(|source| PartError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let marks = parse_wide_marks(&content, format, path)?;
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
        .map(|record| WideMark {
            compressed_offset: read_u64(&record[..8]),
            decompressed_offset: read_u64(&record[8..16]),
            rows: (format == WideMarkFormat::Adaptive).then(|| read_u64(&record[16..24])),
        })
        .collect())
}

fn read_u64(bytes: &[u8]) -> u64 {
    let mut value = [0; 8];
    value.copy_from_slice(bytes);
    u64::from_le_bytes(value)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn accepts_empty_marks() {
        for format in [WideMarkFormat::Fixed, WideMarkFormat::Adaptive] {
            assert!(
                parse_wide_marks(&[], format, Path::new("empty"))
                    .unwrap()
                    .is_empty()
            );
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
    fn refuses_compressed_and_compact_mark_formats() {
        for path in [
            "marks.cmrk",
            "marks.cmrk2",
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
