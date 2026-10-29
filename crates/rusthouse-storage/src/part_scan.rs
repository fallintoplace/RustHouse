mod open;

use self::open::*;

use crate::granules::read_numeric_granule_cached;
use crate::numeric::numeric_storage_type;
use crate::scan_layout::ScanLayout;
use crate::{
    CompressedBlockReader, DecodedBlock, NumericColumn, PartError, WideMark, inspect_part,
    read_wide_marks,
};
use std::fs::File;
use std::path::{Path, PathBuf};

const PARALLEL_COLUMN_THRESHOLD: usize = 8;
const PARALLEL_ROW_THRESHOLD: u64 = 4096;
const MAX_OPEN_WORKERS: usize = 4;

struct ColumnStream {
    data_type: String,
    marks: Vec<WideMark>,
    reader: CompressedBlockReader<File>,
    cached: Option<DecodedBlock>,
}

/// Numeric streams are independent after cross-column mark validation.
/// Decode large wide projections on bounded scoped workers in column order.
fn decode_numeric_stream(
    stream: &mut ColumnStream,
    index: usize,
) -> Result<NumericColumn, PartError> {
    read_numeric_granule_cached(
        &mut stream.reader,
        &stream.marks[index],
        &stream.data_type,
        &mut stream.cached,
    )
}

fn decode_numeric_streams(
    streams: &mut [ColumnStream],
    index: usize,
    workers: usize,
) -> Result<Vec<NumericColumn>, PartError> {
    if workers <= 1 || streams.len() < 2 {
        return streams.iter_mut().map(|stream| decode_numeric_stream(stream, index)).collect();
    }
    let width = streams.len().div_ceil(workers);
    let pieces = std::thread::scope(|scope| {
        let handles = streams.chunks_mut(width)
            .map(|chunk| scope.spawn(move || {
                chunk.iter_mut()
                    .map(|stream| decode_numeric_stream(stream, index))
                    .collect::<Result<Vec<_>, PartError>>()
            })).collect::<Vec<_>>();
        handles.into_iter().map(|handle| handle.join().unwrap_or_else(|_| {
            Err(PartError::InvalidScan("numeric granule decoder panicked".into()))
        })).collect::<Vec<_>>()
    });
    let mut columns = Vec::with_capacity(streams.len());
    for piece in pieces {
        columns.extend(piece?);
    }
    Ok(columns)
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
        let mut types = Vec::with_capacity(columns.len());
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
            types.push(column.data_type.as_str());
        }

        let mut streams = Vec::with_capacity(columns.len());
        if metadata.rows != 0 {
            let workers = std::thread::available_parallelism()
                .map_or(1, |parallelism| parallelism.get())
                .min(MAX_OPEN_WORKERS);
            if columns.len() >= PARALLEL_COLUMN_THRESHOLD && workers > 1 {
                use std::sync::atomic::{AtomicUsize, Ordering};
                let cursor = AtomicUsize::new(0);
                let mut slots: Vec<Option<Result<ColumnStream, PartError>>> =
                    (0..columns.len()).map(|_| None).collect();
                std::thread::scope(|scope| -> Result<(), PartError> {
                    let handles = (0..workers)
                        .map(|_| {
                            let cursor = &cursor;
                            let types = &types;
                            scope.spawn(move || {
                                let mut completed = Vec::new();
                                loop {
                                    let index = cursor.fetch_add(1, Ordering::Relaxed);
                                    if index >= columns.len() {
                                        break;
                                    }
                                    completed.push((
                                        index,
                                        open_numeric_stream(part, &columns[index], types[index]),
                                    ));
                                }
                                completed
                            })
                        })
                        .collect::<Vec<_>>();
                    for handle in handles {
                        let completed = handle.join().map_err(|_| {
                            PartError::InvalidScan("numeric stream opener panicked".into())
                        })?;
                        for (index, result) in completed {
                            slots[index] = Some(result);
                        }
                    }
                    Ok(())
                })?;
                for result in slots {
                    streams.push(result.expect("each stream is claimed once")?);
                }
            } else {
                for (name, data_type) in columns.iter().zip(&types) {
                    streams.push(open_numeric_stream(part, name, data_type)?);
                }
            }
        }
        // Stream contents can open concurrently, but corresponding mark
        // granules must be validated against one shared scan layout.
        let mut layout = ScanLayout::default();
        for (name, stream) in columns.iter().zip(&streams) {
            layout.validate(name, &stream.marks, metadata.rows)?;
        }
        Ok(Self {
            streams,
            rows: metadata.rows,
            marks: layout.mark_count(),
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

    /// Skip whole validated adaptive marks without decompressing their streams.
    /// The caller must still discard any remaining rows inside the next mark.
    /// Legacy marks without row counts cannot be skipped safely.
    pub fn skip_complete_marks(&mut self, max_rows: usize) -> usize {
        let Some(stream) = self.streams.first() else {
            return 0;
        };
        let mut skipped = 0usize;
        while self.next_mark < self.marks {
            let Some(count) = stream.marks[self.next_mark].rows else {
                break;
            };
            let Ok(count) = usize::try_from(count) else {
                break;
            };
            if count == 0 || count > max_rows - skipped {
                break;
            }
            skipped += count;
            self.next_mark += 1;
        }
        skipped
    }

    pub fn next_batch(&mut self) -> Result<Option<Vec<NumericColumn>>, PartError> {
        if self.next_mark == self.marks {
            return Ok(None);
        }
        let index = self.next_mark;
        // Legacy marks have no adaptive row count and must remain readable.
        // Only an explicit terminal zero-row mark is an end sentinel.
        let row_count = self.streams.first()
            .and_then(|stream| stream.marks[index].rows);
        if row_count == Some(0) {
            self.next_mark = self.marks;
            return Ok(None);
        }
        // Small or legacy marks remain serial: worker setup is worthwhile only
        // for large adaptive analytical projections.
        let workers = if row_count.is_some_and(|rows| rows >= PARALLEL_ROW_THRESHOLD)
            && self.streams.len() >= PARALLEL_COLUMN_THRESHOLD
        {
            std::thread::available_parallelism().map_or(1, |n| n.get()).min(MAX_OPEN_WORKERS)
        } else {
            1
        };
        let columns = decode_numeric_streams(&mut self.streams, index, workers)?;
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
    fn rejects_marks_pointing_inside_compressed_blocks() {
        let dir = fixture();
        let mut invalid = marks(4, &[2, 1]);
        invalid[..8].copy_from_slice(&1u64.to_le_bytes());
        invalid[24..32].copy_from_slice(&1u64.to_le_bytes());
        fs::write(dir.join("id.mrk2"), invalid).unwrap();

        assert!(matches!(
            NumericPartReader::open(&dir, &["id".into()]),
            Err(PartError::InvalidMarkOffset { .. })
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

#[cfg(test)]
mod parallel_numeric_open_tests {
    use super::*;
    use crate::{Column, write_wide_part};
    use std::fs;

    #[test]
    fn wide_numeric_projection_matches_all_column_mark_layouts() {
        let part = std::env::temp_dir().join(format!(
            "rusthouse-numeric-wide-open-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&part);
        let columns = (0..10)
            .map(|index| Column {
                name: format!("n{index}"),
                data_type: "UInt64".into(),
            })
            .collect::<Vec<_>>();
        let data = (0..10)
            .map(|index| vec![Some(index.to_string()), Some((index + 200).to_string())])
            .collect::<Vec<_>>();
        write_wide_part(&part, &columns, &data).unwrap();
        let names = columns.iter().map(|column| column.name.clone()).collect::<Vec<_>>();
        let mut reader = NumericPartReader::open(&part, &names).unwrap();
        let batch = reader.next_batch().unwrap().unwrap();
        assert_eq!(batch.len(), 10);
        assert!(batch.iter().all(|column| column.len() == 2));
        assert!(reader.next_batch().unwrap().is_none());
        fs::remove_dir_all(part).unwrap();
    }
}

#[cfg(test)]
mod parallel_numeric_decode_tests {
    use super::*;
    use crate::{Column, write_wide_part};
    use std::fs;

    #[test]
    fn bounded_parallel_numeric_decompress_preserves_all_column_values() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-parallel-numeric-decode-{}", std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let schema = (0..12).map(|n| Column {
            name: format!("measure{n}"), data_type: "UInt64".into(),
        }).collect::<Vec<_>>();
        let rows = (0..1024).map(|row| (0..12)
            .map(|column| Some((row * 100 + column).to_string()))
            .collect::<Vec<_>>()).collect::<Vec<_>>();
        write_wide_part(&root, &schema, &rows).unwrap();
        let projection = schema.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
        let mut serial = NumericPartReader::open(&root, &projection).unwrap();
        let mut parallel = NumericPartReader::open(&root, &projection).unwrap();
        let expected = decode_numeric_streams(&mut serial.streams, 0, 1).unwrap();
        let actual = decode_numeric_streams(&mut parallel.streams, 0, 4).unwrap();
        assert_eq!(expected, actual);
        assert_eq!(actual.len(), 12);
        assert_eq!(actual[0].len(), 1024);
        fs::remove_dir_all(root).unwrap();
    }
}
