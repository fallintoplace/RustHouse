mod open;

use self::open::*;

use crate::granules::read_numeric_granule_cached;
use crate::numeric::numeric_storage_type;
use crate::scan_layout::ScanLayout;
use crate::{
    CompressedBlockReader, DecodedBlock, NumericColumn, PartError, WideMark, inspect_part,
};
use std::fs::File;
use std::path::{Path, PathBuf};

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
        return streams
            .iter_mut()
            .map(|stream| decode_numeric_stream(stream, index))
            .collect();
    }
    let width = streams.len().div_ceil(workers);
    let pieces = std::thread::scope(|scope| {
        let handles = streams
            .chunks_mut(width)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter_mut()
                        .map(|stream| decode_numeric_stream(stream, index))
                        .collect::<Result<Vec<_>, PartError>>()
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| {
                    Err(PartError::InvalidScan(
                        "numeric granule decoder panicked".into(),
                    ))
                })
            })
            .collect::<Vec<_>>()
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
    failed: bool,
    decode_workers: usize,
}

impl NumericPartReader {
    pub fn open(part: impl AsRef<Path>, columns: &[String]) -> Result<Self, PartError> {
        if columns.is_empty() {
            return Err(PartError::InvalidScan(
                "at least one column is required".into(),
            ));
        }
        let part = part.as_ref();
        if crate::is_compact_part(part)? {
            return Err(PartError::InvalidScan(
                "compact MergeTree part requires a compact-part reader".into(),
            ));
        }
        let metadata = inspect_part(part)?;
        // A narrow projection does not need a hash table for every schema
        // field. Wide projections still resolve columns through one index.
        let schema = if columns.len() > 4 {
            let mut index = std::collections::HashMap::with_capacity(metadata.columns.len());
            for column in &metadata.columns {
                index
                    .entry(column.name.as_str())
                    .or_insert(column.data_type.as_str());
            }
            Some(index)
        } else {
            None
        };
        let mut types = Vec::with_capacity(columns.len());
        for name in columns {
            let data_type = if let Some(schema) = &schema {
                schema.get(name.as_str()).copied()
            } else {
                metadata
                    .columns
                    .iter()
                    .find(|column| column.name == *name)
                    .map(|column| column.data_type.as_str())
            }
            .ok_or_else(|| PartError::InvalidScan(format!("unknown column: {name}")))?;
            if !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err(PartError::InvalidScan(format!(
                    "unsupported column filename: {name}"
                )));
            }
            if numeric_storage_type(data_type).is_none() {
                return Err(PartError::InvalidScan(format!(
                    "unsupported numeric column type: {name} {data_type}"
                )));
            }
            types.push(data_type);
        }

        let mut streams = Vec::with_capacity(columns.len());
        if metadata.rows != 0 {
            const PARALLEL_COLUMN_THRESHOLD: usize = 8;
            const MAX_OPEN_WORKERS: usize = 4;
            let workers = std::thread::available_parallelism()
                .map_or(1, |parallelism| parallelism.get())
                .min(MAX_OPEN_WORKERS);
            if columns.len() >= PARALLEL_COLUMN_THRESHOLD && workers > 1 {
                use std::sync::atomic::{AtomicUsize, Ordering};
                let cursor = AtomicUsize::new(0);
                let mut slots: Vec<Option<Result<ColumnStream, PartError>>> =
                    (0..columns.len()).map(|_| None).collect();
                std::thread::scope(|scope| -> Result<(), PartError> {
                    let mut handles = Vec::with_capacity(workers);
                    for _ in 0..workers {
                        let cursor = &cursor;
                        let types = &types;
                        let handle = std::thread::Builder::new()
                            .spawn_scoped(scope, move || {
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
                            .map_err(|error| {
                                PartError::InvalidScan(format!(
                                    "cannot start numeric stream opener: {error}"
                                ))
                            })?;
                        handles.push(handle);
                    }
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
        let decode_workers = if columns.len() >= 8 {
            std::thread::available_parallelism()
                .map_or(1, |count| count.get())
                .min(4)
        } else {
            1
        };
        Ok(Self {
            streams,
            rows: metadata.rows,
            marks: layout.mark_count(),
            next_mark: 0,
            decode_workers,
            failed: false,
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
        if self.failed {
            return Err(PartError::InvalidScan(
                "numeric scan stopped after decoder failure".into(),
            ));
        }
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
        if self.failed {
            return 0;
        }
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
        if self.failed {
            return Err(PartError::InvalidScan(
                "numeric scan stopped after decoder failure".into(),
            ));
        }
        if self.next_mark == self.marks {
            return Ok(None);
        }
        let index = self.next_mark;
        // Legacy marks have no adaptive row count and must remain readable.
        // Only an explicit terminal zero-row mark is an end sentinel.
        let row_count = self
            .streams
            .first()
            .and_then(|stream| stream.marks[index].rows);
        if row_count == Some(0) {
            self.next_mark = self.marks;
            return Ok(None);
        }
        // Small or legacy marks remain serial: worker setup is worthwhile only
        // for large adaptive analytical projections.
        let workers = if row_count.is_some_and(|rows| rows >= 4096) {
            self.decode_workers
        } else {
            1
        };
        // A failed projection may have advanced other independent streams.
        // Once their cursors are out of sync, retrying this reader is unsafe.
        let columns = match decode_numeric_streams(&mut self.streams, index, workers) {
            Ok(columns) => columns,
            Err(error) => {
                self.failed = true;
                return Err(error);
            }
        };
        self.next_mark += 1;
        Ok(Some(columns))
    }
}

#[cfg(test)]
#[path = "part_scan_tests/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "part_scan_tests/parallel_numeric_open_tests.rs"]
mod parallel_numeric_open_tests;

#[cfg(test)]
#[path = "part_scan_tests/parallel_numeric_decode_tests.rs"]
mod parallel_numeric_decode_tests;

#[cfg(test)]
#[path = "part_scan_tests/narrow_numeric_schema_lookup_tests.rs"]
mod narrow_numeric_schema_lookup_tests;
