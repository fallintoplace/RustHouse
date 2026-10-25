use crate::{
    CompressedBlockReader, NumericColumn, PartError, WideMark, inspect_part, read_numeric_granule,
    read_wide_marks,
};
use std::fs::File;
use std::path::Path;

struct ColumnStream {
    data_type: String,
    marks: Vec<WideMark>,
    reader: CompressedBlockReader<File>,
}

pub struct NumericPartReader {
    streams: Vec<ColumnStream>,
    rows: u64,
    marks: usize,
    next_mark: usize,
}

impl NumericPartReader {
    pub fn open(part: impl AsRef<Path>, columns: &[String]) -> Result<Self, PartError> {
        if columns.is_empty() {
            return Err(PartError::InvalidScan(
                "at least one column is required".into(),
            ));
        }
        let part = part.as_ref();
        let metadata = inspect_part(part)?;
        let mut streams = Vec::with_capacity(columns.len());
        let mut baseline: Option<Vec<u64>> = None;

        for name in columns {
            let column = metadata
                .columns
                .iter()
                .find(|column| column.name == *name)
                .ok_or_else(|| PartError::InvalidScan(format!("unknown column: {name}")))?;
            if !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err(PartError::InvalidScan(format!(
                    "unsupported column filename: {name}"
                )));
            }
            if !matches!(
                column.data_type.as_str(),
                "UInt8"
                    | "UInt16"
                    | "UInt32"
                    | "UInt64"
                    | "Int8"
                    | "Int16"
                    | "Int32"
                    | "Int64"
                    | "Float32"
                    | "Float64"
            ) {
                return Err(PartError::InvalidScan(format!(
                    "unsupported numeric column type: {} {}",
                    column.name, column.data_type
                )));
            }

            if metadata.rows == 0 {
                continue;
            }
            let plain_marks = part.join(format!("{name}.mrk2"));
            let compressed_marks = part.join(format!("{name}.cmrk2"));
            let marks_path = if plain_marks.exists() || !compressed_marks.exists() {
                plain_marks
            } else {
                compressed_marks
            };
            let marks = read_wide_marks(&marks_path)?.marks;
            for (index, mark) in marks.iter().enumerate() {
                if mark.rows == Some(0) && index + 1 != marks.len() {
                    return Err(PartError::InvalidScan(format!(
                        "zero-row mark is not terminal for {name}"
                    )));
                }
                if index > 0 {
                    let previous = &marks[index - 1];
                    if (mark.compressed_offset, mark.decompressed_offset)
                        < (previous.compressed_offset, previous.decompressed_offset)
                    {
                        return Err(PartError::InvalidScan(format!(
                            "mark offsets are not ordered for {name}"
                        )));
                    }
                }
            }
            let row_counts = marks
                .iter()
                .map(|mark| {
                    mark.rows.ok_or_else(|| {
                        PartError::InvalidScan("adaptive wide marks are required".into())
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let total = row_counts
                .iter()
                .try_fold(0u64, |sum, rows| sum.checked_add(*rows));
            if total != Some(metadata.rows) {
                return Err(PartError::InvalidScan(format!(
                    "mark row counts do not match count.txt for {name}"
                )));
            }
            if let Some(reference) = &baseline {
                if reference != &row_counts {
                    return Err(PartError::InvalidScan(format!(
                        "granule boundaries differ for {name}"
                    )));
                }
            } else {
                baseline = Some(row_counts);
            }

            streams.push(ColumnStream {
                data_type: column.data_type.clone(),
                marks,
                reader: CompressedBlockReader::open(part.join(format!("{name}.bin")))?,
            });
        }

        Ok(Self {
            streams,
            rows: metadata.rows,
            marks: baseline.as_ref().map_or(0, Vec::len),
            next_mark: 0,
        })
    }

    pub fn rows(&self) -> u64 {
        self.rows
    }

    pub fn next_batch(&mut self) -> Result<Option<Vec<NumericColumn>>, PartError> {
        if self.next_mark == self.marks {
            return Ok(None);
        }
        let mut columns = Vec::with_capacity(self.streams.len());
        for stream in &mut self.streams {
            columns.push(read_numeric_granule(
                &mut stream.reader,
                &stream.marks[self.next_mark],
                &stream.data_type,
            )?);
        }
        self.next_mark += 1;
        Ok(Some(columns))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ch_cityhash102::cityhash128;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    fn path() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-numeric-part-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        dir
    }

    fn block(data: &[u8]) -> Vec<u8> {
        let mut encoded = vec![0x02];
        encoded.extend_from_slice(&(9 + data.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&(data.len() as u32).to_le_bytes());
        encoded.extend_from_slice(data);
        let mut result = u128::from(cityhash128(&encoded)).to_le_bytes().to_vec();
        result.extend(encoded);
        result
    }

    fn marks(width: u64, counts: &[u64]) -> Vec<u8> {
        let mut encoded = Vec::new();
        let mut offset = 0u64;
        for count in counts {
            encoded.extend_from_slice(&0u64.to_le_bytes());
            encoded.extend_from_slice(&offset.to_le_bytes());
            encoded.extend_from_slice(&count.to_le_bytes());
            offset += count * width;
        }
        encoded
    }

    fn fixture() -> PathBuf {
        let dir = path();
        fs::write(dir.join("count.txt"), "3\n").unwrap();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n2 columns:\n\u{0060}id\u{0060} Int32\n\u{0060}price\u{0060} UInt16\n",
        )
        .unwrap();
        let ids: Vec<u8> = [1i32, 2, 3]
            .into_iter()
            .flat_map(i32::to_le_bytes)
            .collect();
        let prices: Vec<u8> = [10u16, 20, 30]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        fs::write(dir.join("id.bin"), block(&ids)).unwrap();
        fs::write(dir.join("price.bin"), block(&prices)).unwrap();
        fs::write(dir.join("id.mrk2"), marks(4, &[2, 1])).unwrap();
        fs::write(dir.join("price.mrk2"), marks(2, &[2, 1])).unwrap();
        dir
    }

    #[test]
    fn reads_multiple_columns_across_aligned_granules() {
        let dir = fixture();
        let mut reader = NumericPartReader::open(&dir, &["price".into(), "id".into()]).unwrap();
        assert_eq!(reader.rows(), 3);
        assert_eq!(
            reader.next_batch().unwrap().unwrap(),
            vec![
                NumericColumn::UInt16(vec![10, 20]),
                NumericColumn::Int32(vec![1, 2])
            ]
        );
        assert_eq!(
            reader.next_batch().unwrap().unwrap(),
            vec![
                NumericColumn::UInt16(vec![30]),
                NumericColumn::Int32(vec![3])
            ]
        );
        assert!(reader.next_batch().unwrap().is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn validates_columns_and_mark_boundaries_before_reading() {
        let dir = fixture();
        assert!(matches!(
            NumericPartReader::open(&dir, &["missing".into()]),
            Err(PartError::InvalidScan(_))
        ));
        fs::write(dir.join("price.mrk2"), marks(2, &[1, 2])).unwrap();
        assert!(matches!(
            NumericPartReader::open(&dir, &["id".into(), "price".into()]),
            Err(PartError::InvalidScan(_))
        ));
        fs::write(dir.join("count.txt"), "4\n").unwrap();
        assert!(matches!(
            NumericPartReader::open(&dir, &["id".into()]),
            Err(PartError::InvalidScan(_))
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_invalid_mark_order_and_nonterminal_zero_rows() {
        let dir = fixture();
        fs::write(dir.join("id.mrk2"), marks(4, &[0, 3])).unwrap();
        assert!(matches!(
            NumericPartReader::open(&dir, &["id".into()]),
            Err(PartError::InvalidScan(_))
        ));

        let mut unordered = marks(4, &[2, 1]);
        unordered[8..16].copy_from_slice(&8u64.to_le_bytes());
        unordered[32..40].copy_from_slice(&0u64.to_le_bytes());
        fs::write(dir.join("id.mrk2"), unordered).unwrap();
        assert!(matches!(
            NumericPartReader::open(&dir, &["id".into()]),
            Err(PartError::InvalidScan(_))
        ));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_corrupt_blocks_during_scan() {
        let dir = fixture();
        let mut data = fs::read(dir.join("id.bin")).unwrap();
        data[0] ^= 1;
        fs::write(dir.join("id.bin"), data).unwrap();
        let mut reader = NumericPartReader::open(&dir, &["id".into()]).unwrap();
        assert!(matches!(
            reader.next_batch(),
            Err(PartError::InvalidCompressedBlock { .. })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_parts_do_not_require_column_files() {
        let dir = path();
        fs::write(dir.join("count.txt"), "0\n").unwrap();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n1 columns:\n\u{0060}id\u{0060} UInt64\n",
        )
        .unwrap();
        let mut reader = NumericPartReader::open(&dir, &["id".into()]).unwrap();
        assert_eq!(reader.rows(), 0);
        assert!(reader.next_batch().unwrap().is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}
