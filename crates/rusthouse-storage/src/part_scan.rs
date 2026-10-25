use crate::granules::read_numeric_granule_cached;
use crate::numeric::numeric_storage_type;
use crate::{
    CompressedBlockReader, DecodedBlock, NumericColumn, PartError, WideMark, inspect_part,
    read_wide_marks,
};
use std::fs::File;
use std::path::Path;

struct ColumnStream {
    data_type: String,
    marks: Vec<WideMark>,
    reader: CompressedBlockReader<File>,
    cached: Option<DecodedBlock>,
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
            if numeric_storage_type(&column.data_type).is_none() {
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
            if plain_marks.exists() && compressed_marks.exists() {
                return Err(PartError::InvalidScan(format!(
                    "multiple mark files for {name}"
                )));
            }
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
                    if mark.rows != Some(0)
                        && (mark.compressed_offset, mark.decompressed_offset)
                            == (previous.compressed_offset, previous.decompressed_offset)
                    {
                        return Err(PartError::InvalidScan(format!(
                            "duplicate live mark offsets for {name}"
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
                cached: None,
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

    pub fn mark_count(&self) -> usize {
        self.marks
    }

    /// Position the next scan batch at a mark without reading earlier granules.
    /// The index may equal mark_count() to seek to the end of the part.
    pub fn seek_to_mark(&mut self, index: usize) -> Result<(), PartError> {
        if index > self.marks {
            return Err(PartError::InvalidGranuleIndex {
                index,
                marks: self.marks,
            });
        }
        self.next_mark = index;
        Ok(())
    }

    pub fn next_batch(&mut self) -> Result<Option<Vec<NumericColumn>>, PartError> {
        if self.next_mark == self.marks {
            return Ok(None);
        }
        let mut columns = Vec::with_capacity(self.streams.len());
        for stream in &mut self.streams {
            columns.push(read_numeric_granule_cached(
                &mut stream.reader,
                &stream.marks[self.next_mark],
                &stream.data_type,
                &mut stream.cached,
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
    fn seeks_to_marks_and_can_rescan_previous_batches() {
        let dir = fixture();
        let mut reader = NumericPartReader::open(&dir, &["id".into(), "price".into()]).unwrap();
        assert_eq!(reader.mark_count(), 2);

        reader.seek_to_mark(1).unwrap();
        assert_eq!(
            reader.next_batch().unwrap().unwrap(),
            vec![
                NumericColumn::Int32(vec![3]),
                NumericColumn::UInt16(vec![30])
            ]
        );
        assert!(reader.next_batch().unwrap().is_none());

        reader.seek_to_mark(0).unwrap();
        assert_eq!(
            reader.next_batch().unwrap().unwrap(),
            vec![
                NumericColumn::Int32(vec![1, 2]),
                NumericColumn::UInt16(vec![10, 20])
            ]
        );
        assert!(matches!(
            reader.seek_to_mark(3),
            Err(PartError::InvalidGranuleIndex { index: 3, marks: 2 })
        ));
        assert_eq!(
            reader.next_batch().unwrap().unwrap(),
            vec![
                NumericColumn::Int32(vec![3]),
                NumericColumn::UInt16(vec![30])
            ]
        );

        reader.seek_to_mark(reader.mark_count()).unwrap();
        assert!(reader.next_batch().unwrap().is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_scan_can_only_seek_to_its_end() {
        let dir = path();
        fs::write(dir.join("count.txt"), "0\n").unwrap();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n1 columns:\n`id` UInt8\n",
        )
        .unwrap();
        let mut reader = NumericPartReader::open(&dir, &["id".into()]).unwrap();
        assert_eq!(reader.mark_count(), 0);
        reader.seek_to_mark(0).unwrap();
        assert!(reader.next_batch().unwrap().is_none());
        assert!(matches!(
            reader.seek_to_mark(1),
            Err(PartError::InvalidGranuleIndex { index: 1, marks: 0 })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_signed_and_unsigned_128_bit_columns() {
        let dir = path();
        fs::write(dir.join("count.txt"), "3\n").unwrap();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n2 columns:\n\u{0060}large\u{0060} UInt128\n\u{0060}small\u{0060} Int128\n",
        )
        .unwrap();
        let large = [0u128, u128::MAX, (1u128 << 96) + 1];
        let small = [i128::MIN, -1, i128::MAX];
        let encoded_large = large
            .into_iter()
            .flat_map(u128::to_le_bytes)
            .collect::<Vec<_>>();
        let encoded_small = small
            .into_iter()
            .flat_map(i128::to_le_bytes)
            .collect::<Vec<_>>();
        fs::write(dir.join("large.bin"), block(&encoded_large)).unwrap();
        fs::write(dir.join("small.bin"), block(&encoded_small)).unwrap();
        fs::write(dir.join("large.mrk2"), marks(16, &[2, 1])).unwrap();
        fs::write(dir.join("small.mrk2"), marks(16, &[2, 1])).unwrap();

        let mut reader = NumericPartReader::open(&dir, &["large".into(), "small".into()]).unwrap();
        assert_eq!(
            reader.next_batch().unwrap().unwrap(),
            vec![
                NumericColumn::UInt128(large[..2].to_vec()),
                NumericColumn::Int128(small[..2].to_vec())
            ]
        );
        assert_eq!(
            reader.next_batch().unwrap().unwrap(),
            vec![
                NumericColumn::UInt128(large[2..].to_vec()),
                NumericColumn::Int128(small[2..].to_vec())
            ]
        );
        assert!(reader.next_batch().unwrap().is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_timestamp_column_from_a_wide_part() {
        let dir = path();
        fs::write(dir.join("count.txt"), "2\n").unwrap();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n1 columns:\n`created_at` DateTime64(3, 'UTC')\n",
        )
        .unwrap();
        let values = [-1i64, 1_700_000_000_123];
        let bytes: Vec<_> = values.into_iter().flat_map(i64::to_le_bytes).collect();
        fs::write(dir.join("created_at.bin"), block(&bytes)).unwrap();
        fs::write(dir.join("created_at.mrk2"), marks(8, &[2])).unwrap();

        let mut reader = NumericPartReader::open(&dir, &["created_at".into()]).unwrap();
        assert_eq!(
            reader.next_batch().unwrap().unwrap(),
            vec![NumericColumn::Int64(values.to_vec())]
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
    fn rejects_conflicting_plain_and_compressed_marks() {
        let dir = fixture();
        fs::write(dir.join("id.cmrk2"), b"duplicate").unwrap();

        assert!(matches!(
            NumericPartReader::open(&dir, &["id".into()]),
            Err(PartError::InvalidScan(reason)) if reason.contains("multiple mark files")
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_duplicate_live_mark_positions() {
        let dir = fixture();
        let mut duplicate = marks(4, &[2, 1]);
        duplicate[32..40].copy_from_slice(&0u64.to_le_bytes());
        fs::write(dir.join("id.mrk2"), duplicate).unwrap();

        assert!(matches!(
            NumericPartReader::open(&dir, &["id".into()]),
            Err(PartError::InvalidScan(reason))
                if reason.contains("duplicate live mark offsets")
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
