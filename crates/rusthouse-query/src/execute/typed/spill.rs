//! Disk-backed ORDER BY for typed rows when no bounded top-k is possible.
//! Runs retain original sequence numbers so equal keys preserve input order.

use super::*;
use std::collections::BinaryHeap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

const MAX_BUFFER_ROWS: usize = 8_192;
const MAX_BUFFER_BYTES: usize = 32 * 1024 * 1024;
const MAX_FIELD_BYTES: usize = 32 * 1024 * 1024;
const MAX_TEMP_BYTES: u64 = 32 * 1024 * 1024 * 1024;
const MERGE_FAN_IN: usize = 16;

static NEXT_SORT: AtomicUsize = AtomicUsize::new(0);

fn invalid(message: &'static str) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self, ExecutionError> {
        for _ in 0..64 {
            let sequence = NEXT_SORT.fetch_add(1, AtomicOrdering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rusthouse-typed-sort-{}-{sequence}", std::process::id(),
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(invalid("cannot allocate private sort scratch directory"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct SortEntry {
    row: Row,
    ordinal: usize,
}

fn compare_entries(
    a: &SortEntry,
    b: &SortEntry,
    descending_mask: u8,
    nulls_first_mask: u8,
    nulls_last_mask: u8,
) -> Ordering {
    compare_sort_keys(
        &a.row.sort_keys, &b.row.sort_keys,
        descending_mask, nulls_first_mask, nulls_last_mask,
    ).then_with(|| a.ordinal.cmp(&b.ordinal))
}

fn entry_bytes(row: &Row) -> Result<usize, ExecutionError> {
    let mut cells = row.values.iter().chain(row.sort_keys.iter());
    cells.try_fold(
        std::mem::size_of::<SortEntry>()
            .checked_add((row.values.len() + row.sort_keys.len())
                .checked_mul(std::mem::size_of::<Cell>())
                .ok_or_else(|| invalid("typed sort row-size overflow"))?)
            .ok_or_else(|| invalid("typed sort row-size overflow"))?,
        |used, cell| {
            let extra = match cell {
                Cell::Text(bytes) => bytes.len(),
                Cell::Uuid(value) => value.len(),
                _ => 0,
            };
            used.checked_add(extra).ok_or_else(|| invalid("typed sort row-size overflow"))
        },
    )
}

fn write_cell(output: &mut impl Write, value: &Cell) -> Result<(), ExecutionError> {
    match value {
        Cell::Null => output.write_all(&[0])?,
        Cell::Numeric(Number::Unsigned(v)) => {
            let mut record = [0u8; 17];
            record[0] = 1;
            record[1..].copy_from_slice(&v.to_le_bytes());
            output.write_all(&record)?;
        }
        Cell::Numeric(Number::Signed(v)) => {
            let mut record = [0u8; 17];
            record[0] = 2;
            record[1..].copy_from_slice(&v.to_le_bytes());
            output.write_all(&record)?;
        }
        Cell::Numeric(Number::Float(v)) => {
            let mut record = [0u8; 9];
            record[0] = 3;
            record[1..].copy_from_slice(&v.to_bits().to_le_bytes());
            output.write_all(&record)?;
        }
        Cell::Text(value) => {
            output.write_all(&[4])?;
            write_bytes(output, value)?;
        }
        Cell::Uuid(value) => {
            output.write_all(&[5])?;
            write_bytes(output, value.as_bytes())?;
        }
    }
    Ok(())
}

fn write_bytes(output: &mut impl Write, value: &[u8]) -> Result<(), ExecutionError> {
    let length = u32::try_from(value.len())
        .map_err(|_| invalid("typed sort value exceeds 4 GiB"))?;
    if value.len() > MAX_FIELD_BYTES {
        return Err(invalid("typed sort value exceeds 32 MiB field limit"));
    }
    if value.len() <= 256 {
        let mut frame = [0u8; 260];
        frame[..4].copy_from_slice(&length.to_le_bytes());
        frame[4..4 + value.len()].copy_from_slice(value);
        output.write_all(&frame[..4 + value.len()])?;
        return Ok(());
    }
    output.write_all(&length.to_le_bytes())?;
    output.write_all(value)?;
    Ok(())
}

fn read_cell(input: &mut impl Read) -> Result<Cell, ExecutionError> {
    let mut tag = [0u8; 1];
    input.read_exact(&mut tag)?;
    Ok(match tag[0] {
        0 => Cell::Null,
        1 => {
            let mut data = [0u8; 16];
            input.read_exact(&mut data)?;
            Cell::Numeric(Number::Unsigned(u128::from_le_bytes(data)))
        }
        2 => {
            let mut data = [0u8; 16];
            input.read_exact(&mut data)?;
            Cell::Numeric(Number::Signed(i128::from_le_bytes(data)))
        }
        3 => {
            let mut data = [0u8; 8];
            input.read_exact(&mut data)?;
            Cell::Numeric(Number::Float(f64::from_bits(u64::from_le_bytes(data))))
        }
        4 => Cell::Text(read_bytes(input)?),
        5 => Cell::Uuid(String::from_utf8(read_bytes(input)?)
            .map_err(|_| invalid("invalid UUID bytes in typed sort run"))?),
        _ => return Err(invalid("invalid typed sort cell tag")),
    })
}

fn read_bytes(input: &mut impl Read) -> Result<Vec<u8>, ExecutionError> {
    let mut size = [0u8; 4];
    input.read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    if size > MAX_FIELD_BYTES {
        return Err(invalid("oversized typed sort run value"));
    }
    let mut data = vec![0u8; size];
    input.read_exact(&mut data)?;
    Ok(data)
}

fn write_entry(output: &mut impl Write, entry: &SortEntry) -> Result<(), ExecutionError> {
    let ordinal = u64::try_from(entry.ordinal)
        .map_err(|_| invalid("typed sort sequence overflow"))?;
    let values = u16::try_from(entry.row.values.len())
        .map_err(|_| invalid("too many typed sort projection columns"))?;
    let sort_keys = u16::try_from(entry.row.sort_keys.len())
        .map_err(|_| invalid("too many typed sort keys"))?;
    let mut header = [0u8; 12];
    header[..8].copy_from_slice(&ordinal.to_le_bytes());
    header[8..10].copy_from_slice(&values.to_le_bytes());
    header[10..12].copy_from_slice(&sort_keys.to_le_bytes());
    output.write_all(&header)?;
    for value in entry.row.values.iter().chain(entry.row.sort_keys.iter()) {
        write_cell(output, value)?;
    }
    Ok(())
}

fn read_entry(input: &mut impl Read) -> Result<Option<SortEntry>, ExecutionError> {
    let mut header = [0u8; 12];
    if input.read(&mut header[..1])? == 0 {
        return Ok(None);
    }
    input.read_exact(&mut header[1..])?;
    let ordinal: [u8; 8] = header[..8].try_into().expect("typed sort ordinal width");
    let values = u16::from_le_bytes([header[8], header[9]]) as usize;
    let keys = u16::from_le_bytes([header[10], header[11]]) as usize;
    if values > 4096 || keys > 8 {
        return Err(invalid("typed sort run has invalid projection width"));
    }
    let mut projected = Vec::with_capacity(values);
    let mut sorted = Vec::with_capacity(keys);
    for _ in 0..values {
        projected.push(read_cell(input)?);
    }
    for _ in 0..keys {
        sorted.push(read_cell(input)?);
    }
    Ok(Some(SortEntry {
        row: Row { values: projected, sort_keys: sorted },
        ordinal: usize::try_from(u64::from_le_bytes(ordinal))
            .map_err(|_| invalid("typed sort run sequence exceeds address space"))?,
    }))
}

struct HeapItem {
    entry: SortEntry,
    source: usize,
    descending: u8,
    nulls_first: u8,
    nulls_last: u8,
}

impl PartialEq for HeapItem {
    fn eq(&self, other: &Self) -> bool { self.cmp(other).is_eq() }
}
impl Eq for HeapItem {}
impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}
impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_entries(
            &other.entry, &self.entry,
            self.descending, self.nulls_first, self.nulls_last,
        ).then_with(|| other.source.cmp(&self.source))
    }
}

pub(super) struct TypedExternalSort {
    buffer: Vec<SortEntry>,
    buffer_bytes: usize,
    runs: Vec<PathBuf>,
    scratch: Option<Scratch>,
    files_created: u64,
    disk_bytes: u64,
    ordinal: usize,
    descending: u8,
    nulls_first: u8,
    nulls_last: u8,
}

impl TypedExternalSort {
    pub(super) fn new(descending: u8, nulls_first: u8, nulls_last: u8) -> Self {
        Self {
            buffer: Vec::new(),
            buffer_bytes: 0,
            runs: Vec::new(),
            scratch: None,
            files_created: 0,
            disk_bytes: 0,
            ordinal: 0,
            descending,
            nulls_first,
            nulls_last,
        }
    }

    pub(super) fn push(&mut self, row: Row) -> Result<(), ExecutionError> {
        let bytes = entry_bytes(&row)?;
        if bytes > MAX_BUFFER_BYTES {
            return Err(invalid("typed sort row exceeds 32 MiB memory budget"));
        }
        if self.buffer.len() >= MAX_BUFFER_ROWS ||
            self.buffer_bytes.checked_add(bytes).is_none_or(|n| n > MAX_BUFFER_BYTES)
        {
            self.flush()?;
        }
        self.buffer_bytes = self.buffer_bytes.checked_add(bytes)
            .ok_or_else(|| invalid("typed sort memory accounting overflow"))?;
        let ordinal = self.ordinal;
        self.ordinal = self.ordinal.checked_add(1)
            .ok_or_else(|| invalid("typed sort sequence overflow"))?;
        self.buffer.push(SortEntry { row, ordinal });
        Ok(())
    }

    fn sort_buffer(&mut self) {
        let (descending, nulls_first, nulls_last) =
            (self.descending, self.nulls_first, self.nulls_last);
        self.buffer.sort_unstable_by(|a, b| {
            compare_entries(a, b, descending, nulls_first, nulls_last)
        });
    }

    fn next_path(&mut self) -> Result<PathBuf, ExecutionError> {
        if self.scratch.is_none() {
            self.scratch = Some(Scratch::new()?);
        }
        let ordinal = self.files_created;
        self.files_created = self.files_created.checked_add(1)
            .ok_or_else(|| invalid("typed sort run count overflow"))?;
        Ok(self.scratch.as_ref().unwrap().0.join(format!("run-{ordinal:016}.bin")))
    }

    fn charge_file(&mut self, path: &Path) -> Result<(), ExecutionError> {
        let bytes = fs::metadata(path)?.len();
        let next = self.disk_bytes.checked_add(bytes)
            .ok_or_else(|| invalid("typed sort spill byte overflow"))?;
        if next > MAX_TEMP_BYTES {
            return Err(invalid("typed sort spill exceeds 32 GiB budget"));
        }
        self.disk_bytes = next;
        Ok(())
    }

    fn drop_file(&mut self, path: &Path) -> Result<(), ExecutionError> {
        let bytes = fs::metadata(path)?.len();
        fs::remove_file(path)?;
        self.disk_bytes = self.disk_bytes.saturating_sub(bytes);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), ExecutionError> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        self.sort_buffer();
        let path = self.next_path()?;
        let mut output = BufWriter::with_capacity(64 * 1024, File::create(&path)?);
        for entry in &self.buffer {
            write_entry(&mut output, entry)?;
        }
        output.flush()?;
        drop(output);
        self.charge_file(&path)?;
        self.buffer.clear();
        self.buffer_bytes = 0;
        self.runs.push(path);
        Ok(())
    }

    fn merge_group<F>(
        &self,
        paths: &[PathBuf],
        mut emit: F,
    ) -> Result<(), ExecutionError>
    where F: FnMut(SortEntry) -> Result<bool, ExecutionError>,
    {
        if let [path] = paths {
            // A single pre-sorted run does not need a heap or comparisons.
            let mut source = BufReader::with_capacity(64 * 1024, File::open(path)?);
            while let Some(entry) = read_entry(&mut source)? {
                if !emit(entry)? { break; }
            }
            return Ok(());
        }
        let mut sources = paths.iter().map(|path| {
            File::open(path)
                .map(|file| BufReader::with_capacity(64 * 1024, file))
                .map_err(ExecutionError::from)
        }).collect::<Result<Vec<_>, _>>()?;
        let mut heap = BinaryHeap::new();
        for (index, source) in sources.iter_mut().enumerate() {
            if let Some(entry) = read_entry(source)? {
                heap.push(HeapItem {
                    entry, source: index, descending: self.descending,
                    nulls_first: self.nulls_first, nulls_last: self.nulls_last,
                });
            }
        }
        while let Some(item) = heap.pop() {
            let source = item.source;
            if !emit(item.entry)? {
                // A globally merged LIMIT needs no further read or decode.
                return Ok(());
            }
            if let Some(entry) = read_entry(&mut sources[source])? {
                heap.push(HeapItem {
                    entry, source, descending: self.descending,
                    nulls_first: self.nulls_first, nulls_last: self.nulls_last,
                });
            }
        }
        Ok(())
    }

    fn compact(&mut self) -> Result<(), ExecutionError> {
        while self.runs.len() > MERGE_FAN_IN {
            let current = std::mem::take(&mut self.runs);
            let mut next = Vec::with_capacity(current.len().div_ceil(MERGE_FAN_IN));
            for chunk in current.chunks(MERGE_FAN_IN) {
                if chunk.len() == 1 {
                    next.push(chunk[0].clone());
                    continue;
                }
                let current_bytes = chunk.iter().try_fold(0u64, |total, path| {
                    total.checked_add(fs::metadata(path)?.len())
                        .ok_or_else(|| invalid("typed sort merge size overflow"))
                })?;
                if self.disk_bytes.checked_add(current_bytes)
                    .is_none_or(|n| n > MAX_TEMP_BYTES)
                {
                    return Err(invalid("typed sort needs more than 32 GiB scratch"));
                }
                let merged = self.next_path()?;
                {
                    let mut writer = BufWriter::with_capacity(64 * 1024, File::create(&merged)?);
                    self.merge_group(chunk, |entry| {
                        write_entry(&mut writer, &entry)?;
                        Ok(true)
                    })?;
                    writer.flush()?;
                }
                self.charge_file(&merged)?;
                for path in chunk {
                    self.drop_file(path)?;
                }
                next.push(merged);
            }
            self.runs = next;
        }
        Ok(())
    }

    pub(super) fn write<W: Write>(
        mut self,
        output: &mut W,
        data_types: &[&str],
        offset: usize,
        limit: usize,
    ) -> Result<(), ExecutionError> {
        if self.runs.is_empty() {
            self.sort_buffer();
            for entry in self.buffer.into_iter().skip(offset).take(limit) {
                write_row(output, &entry.row.values, data_types)?;
            }
            return Ok(());
        }
        self.flush()?;
        self.compact()?;
        let mut seen = 0usize;
        let mut emitted = 0usize;
        self.merge_group(&self.runs, |entry| {
            if seen >= offset && emitted < limit {
                write_row(output, &entry.row.values, data_types)?;
                emitted += 1;
            }
            seen = seen.checked_add(1)
                .ok_or_else(|| invalid("typed sort merge row count overflow"))?;
            Ok(emitted < limit)
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(text: &str, value: u64) -> Row {
        Row {
            values: vec![Cell::Text(text.as_bytes().to_vec()), Cell::Numeric(Number::Unsigned(value as u128))],
            sort_keys: vec![Cell::Numeric(Number::Unsigned(value as u128))],
        }
    }

    #[test]
    fn single_spill_run_preserves_offset_limit_and_order() {
        let mut sorter = TypedExternalSort::new(0, 0, 0);
        for value in (0..32).rev() {
            sorter.push(row("single-run", value)).unwrap();
        }
        sorter.flush().unwrap();
        assert_eq!(sorter.runs.len(), 1);
        let mut result = Vec::new();
        sorter.write(&mut result, &["String", "UInt64"], 19, 3).unwrap();
        assert_eq!(String::from_utf8(result).unwrap(),
            "single-run\t19\nsingle-run\t20\nsingle-run\t21\n");
    }

    #[test]
    fn short_string_spill_frame_uses_one_write() {
        struct Calls { count: usize, bytes: Vec<u8> }
        impl Write for Calls {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.count += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let mut output = Calls { count: 0, bytes: Vec::new() };
        write_bytes(&mut output, b"hello").unwrap();
        let mut expected = 5u32.to_le_bytes().to_vec();
        expected.extend_from_slice(b"hello");
        assert_eq!(output.count, 1);
        assert_eq!(output.bytes, expected);
    }

    #[test]
    fn spill_round_trip_retains_nullable_strings_uuids_and_numbers() {
        let mut input = Vec::new();
        let entry = SortEntry {
            row: Row {
                values: vec![
                    Cell::Null, Cell::Text(b"hello\tworld".to_vec()),
                    Cell::Uuid("61f0c404-5cb3-11e7-907b-a6006ad3dba0".into()),
                    Cell::Numeric(Number::Signed(-5)),
                    Cell::Numeric(Number::Float(1.5)),
                ],
                sort_keys: vec![Cell::Text(b"z".to_vec())],
            },
            ordinal: 3,
        };
        write_entry(&mut input, &entry).unwrap();
        let recovered = read_entry(&mut input.as_slice()).unwrap().unwrap();
        assert_eq!(recovered.ordinal, 3);
        assert_eq!(recovered.row.values.len(), 5);
        assert!(matches!(&recovered.row.values[1], Cell::Text(value) if value == b"hello\tworld"));
        assert!(matches!(&recovered.row.values[2], Cell::Uuid(value) if value.starts_with("61f0")));
        assert!(matches!(&recovered.row.values[3], Cell::Numeric(Number::Signed(-5))));
        assert!(matches!(&recovered.row.values[4], Cell::Numeric(Number::Float(1.5))));
        let mut empty: &[u8] = &[];
        assert!(read_entry(&mut empty).unwrap().is_none());
    }

    #[test]
    fn many_runs_merge_with_stable_keys_and_global_limit_offset() {
        let mut sorter = TypedExternalSort::new(0, 0, 0);
        for index in 0..(MAX_BUFFER_ROWS * (MERGE_FAN_IN + 1)) {
            sorter.push(row(&format!("key-{index:06}"), (index % 10) as u64)).unwrap();
        }
        let mut result = Vec::new();
        sorter.write(&mut result, &["String", "UInt64"], 5, 3).unwrap();
        let text = String::from_utf8(result).unwrap();
        // Equal keys preserve input order, and OFFSET is applied after global sort.
        assert!(text.starts_with("key-000050\t0\nkey-000060\t0\nkey-000070\t0\n"), "{text}");
    }

    #[test]
    fn global_limit_stops_merging_before_truncated_unneeded_tail() {
        use std::fs::OpenOptions;
        let mut sorter = TypedExternalSort::new(0, 0, 0);
        for index in 0..(MAX_BUFFER_ROWS + 2) {
            sorter.push(row("a", index as u64)).unwrap();
        }
        sorter.flush().unwrap();
        // The incomplete final record would fail if a LIMIT 1 query reads
        // the entire run. Sorted order makes this file's first entry smallest.
        let first = sorter.runs[0].clone();
        OpenOptions::new().append(true).open(&first).unwrap()
            .write_all(&[0x11]).unwrap();
        let mut result = Vec::new();
        sorter.write(&mut result, &["String", "UInt64"], 0, 1).unwrap();
        assert_eq!(String::from_utf8(result).unwrap(), "a\t0\n");
    }

    #[test]
    fn read_entry_rejects_truncated_rows_and_invalid_tags() {
        let mut data = Vec::new();
        write_entry(&mut data, &SortEntry { row: row("a", 5), ordinal: 1 }).unwrap();
        let short = &data[..data.len() - 1];
        assert!(read_entry(&mut short.as_ref()).is_err());
        let mut malformed = data;
        malformed[12] = 99;
        assert!(read_entry(&mut malformed.as_slice()).is_err());
    }
}
