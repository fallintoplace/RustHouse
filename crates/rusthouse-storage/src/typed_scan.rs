//! Read typed batches from immutable wide MergeTree parts.
//! This reader is deliberately separate from the optimized numeric-only scan.
use crate::granules::{read_mark_bytes_cached, read_numeric_granule_cached};
use crate::numeric::numeric_storage_type;
use crate::scan_layout::ScanLayout;
use crate::{
    CompressedBlockReader, DecodedBlock, NumericColumn, PartError, StringColumn, UuidValue,
    WideMark, inspect_part, read_legacy_strings, read_uuid_values, read_wide_marks,
    validate_wide_marks,
};
use crate::{inspect_query_source, query_parts};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

pub enum TypedColumn {
    Numeric(NumericColumn),
    Text(StringColumn),
    Uuid(Vec<UuidValue>),
    NullableNumeric {
        values: NumericColumn,
        nulls: Vec<u8>,
    },
    NullableText {
        values: StringColumn,
        nulls: Vec<u8>,
    },
}

impl TypedColumn {
    pub fn len(&self) -> usize {
        match self {
            Self::Numeric(values) => values.len(),
            Self::Text(values) => values.len(),
            Self::Uuid(values) => values.len(),
            Self::NullableNumeric { nulls, .. } | Self::NullableText { nulls, .. } => nulls.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

struct Stream {
    marks: Vec<WideMark>,
    reader: CompressedBlockReader<File>,
    cached: Option<DecodedBlock>,
}

impl Stream {
    fn open(path: &Path, name: &str) -> Result<Self, PartError> {
        let uncompressed_marks = path.join(format!("{name}.mrk2"));
        let compressed_marks = path.join(format!("{name}.cmrk2"));
        if uncompressed_marks.exists() && compressed_marks.exists() {
            return Err(PartError::InvalidScan(format!(
                "ambiguous mark files for {name}"
            )));
        }
        let mark_path = if uncompressed_marks.exists() || !compressed_marks.exists() {
            uncompressed_marks
        } else {
            compressed_marks
        };
        let marks = read_wide_marks(&mark_path)?;
        let data_path = path.join(format!("{name}.bin"));
        validate_wide_marks(&marks, CompressedBlockReader::open(&data_path)?)?;
        Ok(Self {
            marks: marks.marks,
            reader: CompressedBlockReader::open(data_path)?,
            cached: None,
        })
    }

    fn read_fixed(&mut self, index: usize, bytes: usize) -> Result<Vec<u8>, PartError> {
        read_mark_bytes_cached(
            &mut self.reader,
            &self.marks[index],
            bytes,
            &mut self.cached,
        )
    }
}

/// Read logical variable-width bytes across verified compressed blocks.
struct MarkStream<'a> {
    stream: &'a mut Stream,
    block: Option<DecodedBlock>,
    offset: usize,
}

impl<'a> MarkStream<'a> {
    fn open(stream: &'a mut Stream, index: usize) -> Result<Self, PartError> {
        let mark = &stream.marks[index];
        let offset = usize::try_from(mark.decompressed_offset).map_err(|_| {
            PartError::InvalidScan("String mark offset exceeds address space".into())
        })?;
        stream.reader.seek_to_block(mark.compressed_offset)?;
        let block = stream
            .reader
            .next_decoded()
            .ok_or_else(|| PartError::InvalidScan("String stream ends at mark".into()))??;
        if offset >= block.data.len() {
            return Err(PartError::InvalidScan(
                "String mark offset exceeds block".into(),
            ));
        }
        Ok(Self {
            stream,
            block: Some(block),
            offset,
        })
    }
}

impl Read for MarkStream<'_> {
    fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
        if destination.is_empty() {
            return Ok(0);
        }
        loop {
            if let Some(block) = &self.block {
                if self.offset < block.data.len() {
                    let read = destination.len().min(block.data.len() - self.offset);
                    destination[..read]
                        .copy_from_slice(&block.data[self.offset..self.offset + read]);
                    self.offset += read;
                    return Ok(read);
                }
            }
            self.block = match self.stream.reader.next_decoded() {
                None => return Ok(0),
                Some(Ok(block)) => Some(block),
                Some(Err(error)) => return Err(io::Error::other(error.to_string())),
            };
            self.offset = 0;
        }
    }
}

enum StorageKind {
    Numeric(String),
    String,
    Uuid,
    NullableNumeric(String),
    NullableString,
}

struct ColumnReader {
    kind: StorageKind,
    data: Stream,
    null_map: Option<Stream>,
}

pub struct TypedPartReader {
    columns: Vec<ColumnReader>,
    marks: usize,
    next_mark: usize,
}

impl TypedPartReader {
    pub fn open(part: impl AsRef<Path>, names: &[String]) -> Result<Self, PartError> {
        let part = part.as_ref();
        let metadata = inspect_part(part)?;
        if names.is_empty() {
            return Err(PartError::InvalidScan(
                "typed scan requires at least one column".into(),
            ));
        }
        let mut columns = Vec::with_capacity(names.len());
        let mut layout = ScanLayout::default();
        for name in names {
            if !name.bytes().all(|v| v.is_ascii_alphanumeric() || v == b'_') {
                return Err(PartError::InvalidScan("unsafe column filename".into()));
            }
            let data_type = metadata
                .columns
                .iter()
                .find(|column| column.name == *name)
                .ok_or_else(|| PartError::InvalidScan(format!("unknown column: {name}")))?
                .data_type
                .as_str();
            let (kind, nullable) = match data_type {
                "String" => (StorageKind::String, false),
                "UUID" => (StorageKind::Uuid, false),
                "Nullable(String)" => (StorageKind::NullableString, true),
                other if other.starts_with("Nullable(") && other.ends_with(')') => {
                    let inner = &other[9..other.len() - 1];
                    if numeric_storage_type(inner).is_none() {
                        return Err(PartError::InvalidScan(format!(
                            "unsupported typed scan type: {other}"
                        )));
                    }
                    (StorageKind::NullableNumeric(inner.to_owned()), true)
                }
                other if numeric_storage_type(other).is_some() => {
                    (StorageKind::Numeric(other.to_owned()), false)
                }
                other => {
                    return Err(PartError::InvalidScan(format!(
                        "unsupported typed scan type: {other}"
                    )));
                }
            };
            if metadata.rows == 0 {
                continue;
            }
            let data = Stream::open(part, name)?;
            let null_map = nullable
                .then(|| Stream::open(part, &format!("{name}.null")))
                .transpose()?;
            layout.validate(name, &data.marks, metadata.rows)?;
            if let Some(nulls) = &null_map {
                layout.validate(&format!("{name}.null"), &nulls.marks, metadata.rows)?;
            }
            columns.push(ColumnReader {
                kind,
                data,
                null_map,
            });
        }
        Ok(Self {
            columns,
            marks: layout.mark_count(),
            next_mark: 0,
        })
    }

    pub fn next_batch(&mut self) -> Result<Option<Vec<TypedColumn>>, PartError> {
        if self.next_mark >= self.marks {
            return Ok(None);
        }
        let index = self.next_mark;
        if self.columns[0].data.marks[index].rows == Some(0) {
            // A terminal zero-row mark is metadata only, not a data granule.
            self.next_mark = self.marks;
            return Ok(None);
        }
        let mut batch = Vec::with_capacity(self.columns.len());
        for column in &mut self.columns {
            let mark = &column.data.marks[index];
            let rows = usize::try_from(mark.rows.ok_or_else(|| {
                PartError::InvalidScan("typed scan requires adaptive marks".into())
            })?)
            .map_err(|_| PartError::InvalidScan("row count too large".into()))?;
            let nulls = column
                .null_map
                .as_mut()
                .map(|stream| stream.read_fixed(index, rows))
                .transpose()?;
            let values = match &column.kind {
                StorageKind::Numeric(kind) | StorageKind::NullableNumeric(kind) => {
                    TypedColumn::Numeric(read_numeric_granule_cached(
                        &mut column.data.reader,
                        mark,
                        kind,
                        &mut column.data.cached,
                    )?)
                }
                StorageKind::String | StorageKind::NullableString => {
                    let mut stream = MarkStream::open(&mut column.data, index)?;
                    TypedColumn::Text(
                        read_legacy_strings(&mut stream, rows)
                            .map_err(|error| PartError::InvalidScan(error.to_string()))?,
                    )
                }
                StorageKind::Uuid => {
                    let bytes = column.data.read_fixed(
                        index,
                        rows.checked_mul(16).ok_or_else(|| {
                            PartError::InvalidScan("UUID granule too large".into())
                        })?,
                    )?;
                    TypedColumn::Uuid(
                        read_uuid_values(&mut bytes.as_slice(), rows)
                            .map_err(|error| PartError::InvalidScan(error.to_string()))?,
                    )
                }
            };
            let values = match (values, nulls) {
                (TypedColumn::Numeric(values), Some(nulls)) => {
                    TypedColumn::NullableNumeric { values, nulls }
                }
                (TypedColumn::Text(values), Some(nulls)) => {
                    TypedColumn::NullableText { values, nulls }
                }
                (other, None) => other,
                _ => return Err(PartError::InvalidScan("unsupported null map".into())),
            };
            if values.len() != rows {
                return Err(PartError::InvalidScan(
                    "typed batch row count mismatch".into(),
                ));
            }
            batch.push(values);
        }
        self.next_mark += 1;
        Ok(Some(batch))
    }
}

/// Read typed rows from one part or a frozen multi-part snapshot.
/// Only one part's file streams are held open at a time.
pub struct TypedQueryReader {
    parts: Vec<PathBuf>,
    columns: Vec<String>,
    next_part: usize,
    current: Option<TypedPartReader>,
}

impl TypedQueryReader {
    pub fn open(path: &Path, columns: &[String]) -> Result<Self, PartError> {
        let parts = query_parts(path)?;
        let metadata = inspect_query_source(path)?;
        for name in columns {
            if !metadata.columns.iter().any(|column| &column.name == name) {
                return Err(PartError::InvalidScan(format!("unknown column: {name}")));
            }
        }
        Ok(Self {
            parts,
            columns: columns.to_vec(),
            next_part: 0,
            current: None,
        })
    }

    pub fn next_batch(&mut self) -> Result<Option<Vec<TypedColumn>>, PartError> {
        loop {
            if let Some(reader) = self.current.as_mut() {
                if let Some(batch) = reader.next_batch()? {
                    return Ok(Some(batch));
                }
                self.current = None;
            }
            if self.next_part == self.parts.len() {
                return Ok(None);
            }
            self.current = Some(TypedPartReader::open(
                &self.parts[self.next_part],
                &self.columns,
            )?);
            self.next_part += 1;
        }
    }
}
