//! Read typed batches from immutable wide MergeTree parts.
//! This reader is deliberately separate from the optimized numeric-only scan.
use crate::granules::{read_mark_bytes_cached, read_numeric_granule_cached};
use crate::numeric::numeric_storage_type;
use crate::query_parts;
use crate::scan_layout::ScanLayout;
use crate::{
    CompressedBlockReader, DecodedBlock, NumericColumn, PartError, PartMetadata, StringColumn, UuidValue,
    WideMark, inspect_part, read_legacy_strings, read_uuid_values, read_wide_marks,
};
use crate::mark_validation::validate_wide_marks_reader;
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
        let mut reader = CompressedBlockReader::open(&data_path)?;
        validate_wide_marks_reader(&marks, &mut reader)?;
        reader.seek_to_block(0)?;
        Ok(Self {
            marks: marks.marks,
            reader,
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
        // Adjacent String marks can point into the same compressed block.
        // Reuse its verified decoded bytes instead of decompressing them again.
        let block = match stream
            .cached
            .take()
            .filter(|block| block.block.offset == mark.compressed_offset)
        {
            Some(block) => block,
            None => {
                stream.reader.seek_to_block(mark.compressed_offset)?;
                stream
                    .reader
                    .next_decoded()
                    .ok_or_else(|| PartError::InvalidScan("String stream ends at mark".into()))??
            }
        };
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

impl Drop for MarkStream<'_> {
    fn drop(&mut self) {
        // Preserve the last decoded block for the following adaptive mark.
        self.stream.cached = self.block.take();
    }
}

enum StorageKind {
    Numeric(String),
    String,
    Uuid,
    NullableNumeric(String),
    NullableString,
}

impl StorageKind {
    fn from_data_type(data_type: &str) -> Result<Self, PartError> {
        match data_type {
            "String" => Ok(Self::String),
            "UUID" => Ok(Self::Uuid),
            "Nullable(String)" => Ok(Self::NullableString),
            other if other.starts_with("Nullable(") && other.ends_with(')') => {
                let inner = &other[9..other.len() - 1];
                if numeric_storage_type(inner).is_none() {
                    return Err(PartError::InvalidScan(format!(
                        "unsupported typed scan type: {other}"
                    )));
                }
                Ok(Self::NullableNumeric(inner.to_owned()))
            }
            other if numeric_storage_type(other).is_some() => Ok(Self::Numeric(other.to_owned())),
            other => Err(PartError::InvalidScan(format!(
                "unsupported typed scan type: {other}"
            ))),
        }
    }

    fn needs_null_map(&self) -> bool {
        matches!(self, Self::NullableNumeric(_) | Self::NullableString)
    }
}

fn wrap_null_map(values: TypedColumn, nulls: Option<Vec<u8>>) -> Result<TypedColumn, PartError> {
    match (values, nulls) {
        (TypedColumn::Numeric(values), Some(nulls)) => {
            Ok(TypedColumn::NullableNumeric { values, nulls })
        }
        (TypedColumn::Text(values), Some(nulls)) => Ok(TypedColumn::NullableText { values, nulls }),
        (other, None) => Ok(other),
        _ => Err(PartError::InvalidScan("unsupported null map".into())),
    }
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
            let kind = StorageKind::from_data_type(data_type)?;
            if metadata.rows == 0 {
                continue;
            }
            let data = Stream::open(part, name)?;
            let null_map = kind
                .needs_null_map()
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

    /// Number of validated granules in this immutable part.
    pub fn mark_count(&self) -> usize {
        self.marks
    }

    /// Seek to a validated granule without decoding preceding granules.
    /// The end sentinel (mark_count) is accepted and yields no further batches.
    /// Callers must still evaluate row predicates on returned batches.
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
            let values = wrap_null_map(values, nulls)?;
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
        let metadata = crate::query_source::inspect_parts_metadata(path, &parts)?;
        Self::from_validated_parts(parts, columns, &metadata)
    }

    /// Reuse the planner's validated metadata for an immutable snapshot
    /// instead of inspecting every part again before the typed scan.
    pub fn open_with_metadata(
        path: &Path,
        columns: &[String],
        metadata: &PartMetadata,
    ) -> Result<Self, PartError> {
        if metadata.path.as_path() != path {
            return Err(PartError::InvalidScan(
                "scan metadata belongs to a different snapshot".into(),
            ));
        }
        let parts = query_parts(path)?;
        Self::from_validated_parts(parts, columns, metadata)
    }

    fn from_validated_parts(
        parts: Vec<PathBuf>,
        columns: &[String],
        metadata: &PartMetadata,
    ) -> Result<Self, PartError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_kinds_own_their_null_map_requirements() {
        for kind in [
            "Nullable(String)",
            "Nullable(UInt64)",
            "Nullable(DateTime64(3))",
        ] {
            assert!(StorageKind::from_data_type(kind).unwrap().needs_null_map());
        }
        for kind in ["String", "UUID", "UInt64", "DateTime"] {
            assert!(!StorageKind::from_data_type(kind).unwrap().needs_null_map());
        }
        assert!(StorageKind::from_data_type("Nullable(UUID)").is_err());
    }

    #[test]
    fn adjacent_string_marks_reuse_one_decoded_block() {
        use ch_cityhash102::cityhash128;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rusthouse-typed-string-cache-{}-{}.bin",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let data = b"\x03abc\x03def";
        let mut encoded = vec![0x02];
        encoded.extend_from_slice(&(9u32 + data.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&(data.len() as u32).to_le_bytes());
        encoded.extend_from_slice(data);
        let mut checked = u128::from(cityhash128(&encoded)).to_le_bytes().to_vec();
        checked.extend_from_slice(&encoded);
        std::fs::write(&path, checked).unwrap();

        let mut stream = Stream {
            marks: vec![
                WideMark {
                    compressed_offset: 0,
                    decompressed_offset: 0,
                    rows: Some(1),
                },
                WideMark {
                    compressed_offset: 0,
                    decompressed_offset: 4,
                    rows: Some(1),
                },
            ],
            reader: CompressedBlockReader::open(&path).unwrap(),
            cached: None,
        };
        {
            let mut first = MarkStream::open(&mut stream, 0).unwrap();
            let values = read_legacy_strings(&mut first, 1).unwrap();
            assert_eq!(values.get(0), Some(b"abc".as_slice()));
        }
        assert!(
            stream
                .cached
                .as_ref()
                .is_some_and(|block| block.block.offset == 0)
        );
        {
            let mut second = MarkStream::open(&mut stream, 1).unwrap();
            let values = read_legacy_strings(&mut second, 1).unwrap();
            assert_eq!(values.get(0), Some(b"def".as_slice()));
        }
        assert!(
            stream
                .cached
                .as_ref()
                .is_some_and(|block| block.block.offset == 0)
        );
        drop(stream);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn typed_reader_seeks_to_validated_granule_boundaries() {
        use crate::{Column, write_wide_part};
        use std::fs;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rusthouse-typed-mark-seek-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let part = root.join("all_1_1_0");
        let columns = vec![
            Column {
                name: "id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "note".into(),
                data_type: "Nullable(String)".into(),
            },
        ];
        let rows = (0..2050)
            .map(|id| {
                vec![
                    Some(id.to_string()),
                    if id % 2 == 0 {
                        None
                    } else {
                        Some(format!("row-{id}"))
                    },
                ]
            })
            .collect::<Vec<_>>();
        write_wide_part(&part, &columns, &rows).unwrap();
        let metadata = crate::inspect_query_source(&part).unwrap();
        let mut snapshot = TypedQueryReader::open_with_metadata(
            &part,
            &["id".into(), "note".into()],
            &metadata,
        ).unwrap();
        let mut scanned = 0;
        while let Some(batch) = snapshot.next_batch().unwrap() {
            scanned += batch[0].len();
        }
        assert_eq!(scanned, 2050);
        assert!(matches!(
            TypedQueryReader::open_with_metadata(
                &root,
                &["id".into()],
                &metadata,
            ),
            Err(PartError::InvalidScan(_))
        ));


        let mut reader = TypedPartReader::open(&part, &["id".into(), "note".into()]).unwrap();
        assert_eq!(reader.mark_count(), 3);
        reader.seek_to_mark(2).unwrap();
        let tail = reader.next_batch().unwrap().unwrap();
        match &tail[0] {
            TypedColumn::Numeric(NumericColumn::UInt64(values)) => {
                assert_eq!(values, &[2048, 2049]);
            }
            _ => panic!("expected UInt64"),
        }
        assert_eq!(tail[1].len(), 2);
        assert!(reader.next_batch().unwrap().is_none());

        reader.seek_to_mark(0).unwrap();
        assert_eq!(reader.next_batch().unwrap().unwrap()[0].len(), 1024);
        reader.seek_to_mark(reader.mark_count()).unwrap();
        assert!(reader.next_batch().unwrap().is_none());
        assert!(matches!(
            reader.seek_to_mark(4),
            Err(PartError::InvalidGranuleIndex { index: 4, marks: 3 })
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
