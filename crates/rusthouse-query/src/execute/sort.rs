//! Bounded ORDER BY heap with shared sort-direction metadata.
//! Rows use Arc<[bool]> so multi-key comparisons never allocate a new
//! directions vector for every retained row.
use super::{ExecutionError, Number, compare_numbers};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::cmp::Reverse;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::io::Write;
use std::sync::Arc;

pub(super) const MAX_SORTED_ROWS: usize = 200_000;
const MAX_SORT_BUFFER_BYTES: usize = 64 * 1024 * 1024;
#[cfg(test)]
const SORT_RUN_ROWS: usize = 128;
#[cfg(not(test))]
const SORT_RUN_ROWS: usize = MAX_SORTED_ROWS;
const MERGE_FAN_IN: usize = 32;
const MAX_TEMP_SORT_BYTES: u64 = 32 * 1024 * 1024 * 1024;
static NEXT_SORT: AtomicU64 = AtomicU64::new(0);

fn invalid(message: impl Into<String>) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}

struct SortScratch(PathBuf);
impl SortScratch {
    fn new() -> Result<Self, ExecutionError> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|err| invalid(err.to_string()))?.as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rusthouse-external-sort-{}-{nanos}-{}",
            std::process::id(), NEXT_SORT.fetch_add(1, AtomicOrdering::Relaxed)));
        let mut dir = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            dir.mode(0o700);
        }
        dir.create(&path)?;
        Ok(Self(path))
    }
}
impl Drop for SortScratch {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

#[cfg(test)]
fn write_number(writer: &mut impl Write, number: Number) -> Result<(), ExecutionError> {
    let (tag, payload) = match number {
        Number::Unsigned(v) => (0u8, v.to_le_bytes()),
        Number::Signed(v) => (1u8, v.to_le_bytes()),
        Number::Float(v) => (2u8, (v.to_bits() as u128).to_le_bytes()),
    };
    let mut encoded = [0u8; 17];
    encoded[0] = tag;
    encoded[1..].copy_from_slice(&payload);
    writer.write_all(&encoded)?;
    Ok(())
}

fn decode_number_record(encoded: &[u8]) -> Result<Number, ExecutionError> {
    let payload: [u8; 16] = encoded[1..].try_into().expect("fixed numeric record");
    match encoded[0] {
        0 => Ok(Number::Unsigned(u128::from_le_bytes(payload))),
        1 => Ok(Number::Signed(i128::from_le_bytes(payload))),
        2 => {
            let bits = u64::try_from(u128::from_le_bytes(payload))
                .map_err(|_| invalid("noncanonical external sort Float64 payload"))?;
            Ok(Number::Float(f64::from_bits(bits)))
        },
        _ => Err(invalid("invalid external sort numeric tag")),
    }
}

#[cfg(test)]
fn read_number(reader: &mut impl Read) -> Result<Number, ExecutionError> {
    let mut encoded = [0u8; 17];
    reader.read_exact(&mut encoded)?;
    decode_number_record(&encoded)
}

/// Encode and write an entire sort record with one buffered write, preserving
/// the existing 16-byte header and 17-byte numeric wire format.
fn write_sorted_row_reusing(
    writer: &mut impl Write,
    row: &SortedRow,
    scratch: &mut Vec<u8>,
) -> Result<(), ExecutionError> {
    let index = u64::try_from(row.sequence).map_err(|_| invalid("sort sequence overflow"))?;
    let secondary = u32::try_from(row.extra_keys.len())
        .map_err(|_| invalid("too many ORDER BY keys"))?;
    let width = u32::try_from(row.values.len())
        .map_err(|_| invalid("too many sorted columns"))?;
    scratch.clear();
    scratch.extend_from_slice(&index.to_le_bytes());
    scratch.extend_from_slice(&secondary.to_le_bytes());
    scratch.extend_from_slice(&width.to_le_bytes());
    append_number_record(scratch, row.key);
    for &key in &row.extra_keys {
        append_number_record(scratch, key);
    }
    for &value in &row.values {
        append_number_record(scratch, value);
    }
    writer.write_all(scratch)?;
    Ok(())
}

fn append_number_record(scratch: &mut Vec<u8>, number: Number) {
    let (tag, payload) = match number {
        Number::Unsigned(v) => (0u8, v.to_le_bytes()),
        Number::Signed(v) => (1u8, v.to_le_bytes()),
        Number::Float(v) => (2u8, (v.to_bits() as u128).to_le_bytes()),
    };
    scratch.push(tag);
    scratch.extend_from_slice(&payload);
}

#[cfg(test)]
fn write_sorted_row(writer: &mut impl Write, row: &SortedRow)
    -> Result<(), ExecutionError> {
    write_sorted_row_reusing(writer, row, &mut Vec::new())
}

/// Read one external run row using a reusable wire buffer. A wide
/// projection needs one bulk body read rather than one read per 17-byte
/// key/value. Retain the existing EOF and malformed-width behavior.
fn read_sorted_row_reusing(
    reader: &mut BufReader<File>,
    descending: bool,
    directions: &Arc<[bool]>,
    scratch: &mut Vec<u8>,
) -> Result<Option<SortedRow>, ExecutionError> {
    let mut header = [0u8; 16];
    match reader.read_exact(&mut header[..1]) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    reader.read_exact(&mut header[1..])?;
    let sequence = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        .map_err(|_| invalid("external sort sequence overflow"))?;
    let extra = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let width = u32::from_le_bytes(header[12..].try_into().unwrap()) as usize;
    if extra != directions.len() || width > 128 {
        return Err(invalid("malformed external sort run width"));
    }
    let bytes = extra.checked_add(width).and_then(|n| n.checked_add(1))
        .and_then(|n| n.checked_mul(17))
        .ok_or_else(|| invalid("external sort run size overflow"))?;
    scratch.resize(bytes, 0);
    reader.read_exact(scratch)?;

    let mut records = scratch.chunks_exact(17);
    let key = decode_number_record(records.next().expect("primary sort key"))?;
    let mut extra_keys = Vec::with_capacity(extra);
    for record in records.by_ref().take(extra) {
        extra_keys.push(decode_number_record(record)?);
    }
    let mut values = Vec::with_capacity(width);
    for record in records {
        values.push(decode_number_record(record)?);
    }
    Ok(Some(SortedRow {
        key, extra_keys, values, sequence,
        descending, extra_descending: directions.clone(),
    }))
}

#[cfg(test)]
fn read_sorted_row(
    reader: &mut BufReader<File>,
    descending: bool,
    directions: &Arc<[bool]>,
) -> Result<Option<SortedRow>, ExecutionError> {
    read_sorted_row_reusing(reader, descending, directions, &mut Vec::new())
}

fn merge_runs(
    inputs: &[PathBuf],
    descending: bool,
    directions: &Arc<[bool]>,
    mut output: impl FnMut(&SortedRow) -> Result<bool, ExecutionError>,
) -> Result<(), ExecutionError> {
    // The common one-run case is already ordered and needs no heap.
    if let [path] = inputs {
        let mut source = BufReader::with_capacity(64 * 1024, File::open(path)?);
        let mut scratch = Vec::new();
        let mut current = read_sorted_row_reusing(&mut source, descending, directions, &mut scratch)?;
        while let Some(row) = current {
            // Do not decode the next row when LIMIT has already stopped
            // output. Otherwise check the complete sort tuple before reuse.
            if !output(&row)? { break; }
            current = read_sorted_row_reusing(&mut source, descending, directions, &mut scratch)?;
            if current.as_ref().is_some_and(|next| row.cmp(next) == Ordering::Greater) {
                return Err(invalid("external sort run is not sorted"));
            }
        }
        return Ok(());
    }
    let mut streams = inputs.iter().map(File::open)
        .map(|source| source.map(|file| BufReader::with_capacity(64 * 1024, file)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut heap = BinaryHeap::<Reverse<(SortedRow, usize)>>::new();
    // Each stream keeps its own wire buffer across k-way merge probes.
    let mut scratch = vec![Vec::new(); streams.len()];
    for (index, stream) in streams.iter_mut().enumerate() {
        if let Some(row) = read_sorted_row_reusing(stream, descending, directions, &mut scratch[index])? {
            heap.push(Reverse((row, index)));
        }
    }
    // Replace the winning heap root in place. A pop followed by a push
    // sifts the heap twice for every row in an external ORDER BY merge.
    while let Some(mut top) = heap.peek_mut() {
        let Reverse((row, index)) = &mut *top;
        if !output(row)? { break; }
        if let Some(next) = read_sorted_row_reusing(
            &mut streams[*index], descending, directions, &mut scratch[*index]
        )? {
            if (*row).cmp(&next) == Ordering::Greater {
                return Err(invalid("external sort run is not sorted"));
            }
            *row = next;
        } else {
            std::collections::binary_heap::PeekMut::pop(top);
        }
    }
    Ok(())
}


#[cfg(test)]
mod merge_root_replacement_tests {
    use super::*;

    #[test]
    fn replaces_heap_root_without_changing_global_stable_order() {
        for descending in [false, true] {
            let directions = Arc::<[bool]>::from(Vec::<bool>::new());
            let scratch = SortScratch::new().unwrap();
            let mut paths = Vec::new();
            let mut expected = Vec::new();
            for (shard, mut keys) in [
                vec![0u128, 4, 4, 9],
                vec![0, 2, 4, 7],
                vec![1, 2, 4, 8],
                vec![],
            ].into_iter().enumerate() {
                if descending { keys.reverse(); }
                let path = scratch.0.join(format!("merge-{shard}.bin"));
                let mut writer = BufWriter::new(File::create(&path).unwrap());
                for (position, key) in keys.into_iter().enumerate() {
                    let sequence = shard * 100 + position;
                    let row = SortedRow {
                        key: Number::Unsigned(key),
                        extra_keys: Vec::new(),
                        values: vec![Number::Unsigned(key)],
                        sequence, descending,
                        extra_descending: directions.clone(),
                    };
                    write_sorted_row(&mut writer, &row).unwrap();
                    expected.push((key, sequence));
                }
                writer.flush().unwrap();
                paths.push(path);
            }
            expected.sort_by(|left, right| {
                (if descending { right.0.cmp(&left.0) } else { left.0.cmp(&right.0) })
                    .then_with(|| left.1.cmp(&right.1))
            });
            let mut actual = Vec::new();
            merge_runs(&paths, descending, &directions, |row| {
                let Number::Unsigned(key) = row.key else { panic!("unexpected key") };
                actual.push((key, row.sequence));
                Ok(true)
            }).unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn a_single_run_stops_before_decoding_unused_truncated_rows() {
        let directions = Arc::<[bool]>::from(Vec::<bool>::new());
        let scratch = SortScratch::new().unwrap();
        let path = scratch.0.join("one.bin");
        let mut writer = File::create(&path).unwrap();
        let row = SortedRow {
            key: Number::Unsigned(7),
            extra_keys: Vec::new(),
            values: vec![Number::Unsigned(7)],
            sequence: 0, descending: false,
            extra_descending: directions.clone(),
        };
        write_sorted_row(&mut writer, &row).unwrap();
        writer.write_all(&[1, 2, 3]).unwrap();
        drop(writer);
        let mut seen = 0;
        merge_runs(&[path], false, &directions, |_| {
            seen += 1;
            Ok(false)
        }).unwrap();
        assert_eq!(seen, 1);
    }
}

struct SortedRow {
    key: Number,
    extra_keys: Vec<Number>,
    values: Vec<Number>,
    sequence: usize,
    descending: bool,
    extra_descending: Arc<[bool]>,
}

fn compare_with_direction(left: Number, right: Number, descending: bool) -> Ordering {
    let ordering = compare_numbers(left, right);
    if descending {
        ordering.reverse()
    } else {
        ordering
    }
}

impl PartialEq for SortedRow {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SortedRow {}

impl PartialOrd for SortedRow {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortedRow {
    fn cmp(&self, other: &Self) -> Ordering {
        let ordering = compare_with_direction(self.key, other.key, self.descending);
        if ordering != Ordering::Equal {
            return ordering;
        }
        for ((left, right), descending) in self
            .extra_keys
            .iter()
            .zip(&other.extra_keys)
            .zip(self.extra_descending.iter())
        {
            let comparison = compare_with_direction(*left, *right, *descending);
            if comparison != Ordering::Equal {
                return comparison;
            }
        }
        self.sequence.cmp(&other.sequence)
    }
}

/// Include the row itself and retained numeric vectors, not just row counts.
fn retained_row_size(row: &SortedRow) -> Result<usize, ExecutionError> {
    let numbers = row.extra_keys.capacity().checked_add(row.values.capacity())
        .and_then(|count| count.checked_mul(std::mem::size_of::<Number>()))
        .ok_or_else(|| invalid("SORT memory estimate overflow"))?;
    std::mem::size_of::<SortedRow>().checked_add(numbers)
        .ok_or_else(|| invalid("SORT memory estimate overflow"))
}

fn updated_sort_bytes(current: usize, release: usize, added: usize) -> Result<usize, ExecutionError> {
    let next = current.checked_sub(release)
        .and_then(|previous| previous.checked_add(added))
        .ok_or_else(|| invalid("SORT retained memory accounting overflow"))?;
    if next > MAX_SORT_BUFFER_BYTES {
        return Err(invalid("SORT exceeds 64 MiB retained-row budget"));
    }
    Ok(next)
}

pub(super) struct SortedRows {
    rows: BinaryHeap<SortedRow>,
    requested: usize,
    offset: usize,
    seen: usize,
    descending: bool,
    extra_descending: Arc<[bool]>,
    scratch: Option<SortScratch>,
    run_paths: Vec<PathBuf>,
    next_run: usize,
    temp_bytes: u64,
    retained_bytes: usize,
}

impl SortedRows {
    pub(super) fn new(limit: Option<usize>, descending: bool) -> Self {
        Self {
            // Most ORDER BY queries use a small explicit LIMIT. Reserve
            // the bounded heap once, without eagerly allocating an unbounded sort.
            rows: BinaryHeap::with_capacity(limit.unwrap_or(0).min(1024)),
            requested: limit.unwrap_or(usize::MAX),
            offset: 0,
            seen: 0,
            descending,
            extra_descending: Arc::from(Vec::<bool>::new()),
            scratch: None,
            run_paths: Vec::new(),
            next_run: 0,
            temp_bytes: 0,
            retained_bytes: 0,
        }
    }

    pub(super) fn with_offset(
        limit: Option<usize>,
        offset: usize,
        descending: bool,
    ) -> Result<Self, ExecutionError> {
        let requested = limit
            .map(|limit| {
                limit
                    .checked_add(offset)
                    .ok_or_else(|| ExecutionError::Invalid("LIMIT plus OFFSET overflows".into()))
            })
            .transpose()?
            .unwrap_or(usize::MAX);
        // Construct directly with the page size instead of allocating for
        // LIMIT and then growing immediately to accommodate OFFSET.
        let mut sorted = Self::new(limit.map(|_| requested), descending);
        sorted.requested = requested;
        sorted.offset = offset;
        Ok(sorted)
    }

    pub(super) fn requested(&self) -> usize {
        self.requested
    }

    pub(super) fn offset(&self) -> usize {
        self.offset
    }

    pub(super) fn descending(&self) -> bool {
        self.descending
    }

    pub(super) fn set_extra_directions(&mut self, directions: impl IntoIterator<Item = bool>) {
        self.extra_descending = Arc::from(directions.into_iter().collect::<Vec<_>>());
    }

    fn next_path(&mut self) -> Result<PathBuf, ExecutionError> {
        if self.scratch.is_none() { self.scratch = Some(SortScratch::new()?); }
        let index = self.next_run;
        self.next_run = self.next_run.checked_add(1)
            .ok_or_else(|| invalid("external sort run count overflow"))?;
        Ok(self.scratch.as_ref().unwrap().0.join(format!("run-{index:08}.bin")))
    }

    fn charge_temp(&mut self, path: &Path) -> Result<(), ExecutionError> {
        self.temp_bytes = self.temp_bytes.checked_add(fs::metadata(path)?.len())
            .ok_or_else(|| invalid("external sort temporary byte overflow"))?;
        if self.temp_bytes > MAX_TEMP_SORT_BYTES {
            return Err(invalid("external sort exceeded 32 GiB temporary disk budget"));
        }
        Ok(())
    }

    fn delete_run(&mut self, path: &Path) -> Result<(), ExecutionError> {
        let bytes = fs::metadata(path)?.len();
        fs::remove_file(path)?;
        self.temp_bytes = self.temp_bytes.checked_sub(bytes)
            .ok_or_else(|| invalid("external sort temporary accounting underflow"))?;
        Ok(())
    }

    /// Merging run files only changes row ordering. Their serialized row
    /// bytes remain identical, so input lengths predict output disk usage.
    fn preflight_merge_run(&self, inputs: &[PathBuf]) -> Result<(), ExecutionError> {
        let bytes = inputs.iter().try_fold(0u64, |total, input| {
            total.checked_add(fs::metadata(input)?.len())
                .ok_or_else(|| invalid("external sort temporary byte overflow"))
        })?;
        if self.temp_bytes.checked_add(bytes)
            .is_none_or(|total| total > MAX_TEMP_SORT_BYTES) {
            return Err(invalid("external sort exceeded 32 GiB temporary disk budget"));
        }
        Ok(())
    }

    fn preflight_flush_run(&self) -> Result<(), ExecutionError> {
        // Every row has a fixed 16-byte header and a 17-byte record for
        // each key and projected numeric value. Bound disk before writing.
        let required = self.rows.iter().try_fold(0u64, |total, row| {
            let fields = row.extra_keys.len().checked_add(row.values.len())
                .and_then(|count| count.checked_add(1))
                .ok_or_else(|| invalid("external sort run size overflow"))?;
            let bytes = u64::try_from(fields)
                .ok().and_then(|count| count.checked_mul(17))
                .and_then(|count| count.checked_add(16))
                .ok_or_else(|| invalid("external sort run size overflow"))?;
            total.checked_add(bytes)
                .ok_or_else(|| invalid("external sort run size overflow"))
        })?;
        if self.temp_bytes.checked_add(required)
            .is_none_or(|total| total > MAX_TEMP_SORT_BYTES)
        {
            return Err(invalid("external sort exceeded 32 GiB temporary disk budget"));
        }
        Ok(())
    }

    fn flush_run(&mut self) -> Result<(), ExecutionError> {
        if self.rows.is_empty() { return Ok(()); }
        self.preflight_flush_run()?;
        let path = self.next_path()?;
        let mut writer = BufWriter::with_capacity(64 * 1024, OpenOptions::new()
            .create_new(true).write(true).open(&path)?);
        let mut row_scratch = Vec::new();
        for row in std::mem::take(&mut self.rows).into_sorted_vec() {
            write_sorted_row_reusing(&mut writer, &row, &mut row_scratch)?;
        }
        writer.flush()?;
        self.charge_temp(&path)?;
        self.run_paths.push(path);
        self.retained_bytes = 0;
        Ok(())
    }

    pub(super) fn push(&mut self, key: Number, values: Vec<Number>) -> Result<(), ExecutionError> {
        self.push_with_extra(key, Vec::new(), values)
    }

    /// A later candidate tied with the worst row loses the stable tie-break.
    /// Only bounded heaps can prune: unbounded scans must still enforce
    /// the maximum unsorted-input row count in push_with_extra.
    pub(super) fn would_discard(
        &self,
        key: Number,
        extra_keys: impl IntoIterator<Item = Number>,
    ) -> bool {
        if self.requested > MAX_SORTED_ROWS || self.rows.len() < self.requested {
            return false;
        }
        let Some(worst) = self.rows.peek() else {
            return false;
        };
        let primary = compare_with_direction(key, worst.key, self.descending);
        if primary != Ordering::Equal {
            return primary == Ordering::Greater;
        }
        for ((left, right), descending) in extra_keys
            .into_iter()
            .zip(&worst.extra_keys)
            .zip(self.extra_descending.iter())
        {
            let comparison = compare_with_direction(left, *right, *descending);
            if comparison != Ordering::Equal {
                return comparison == Ordering::Greater;
            }
        }
        true
    }

    pub(super) fn push_with_extra(
        &mut self,
        key: Number,
        extra_keys: Vec<Number>,
        values: Vec<Number>,
    ) -> Result<(), ExecutionError> {
        // Large or unbounded ORDER BY spills sorted runs instead of
        // rejecting a fact table merely because it exceeds 200k rows.
        if self.requested > MAX_SORTED_ROWS && self.rows.len() >= SORT_RUN_ROWS {
            self.flush_run()?;
        }
        let sequence = self.seen;
        self.seen = self
            .seen
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("too many rows to sort".into()))?;
        let retained = if self.requested > MAX_SORTED_ROWS {
            SORT_RUN_ROWS
        } else {
            self.requested.min(MAX_SORTED_ROWS)
        };
        if retained == 0
            || (self.requested <= MAX_SORTED_ROWS
                && self.rows.len() == retained
                && self.would_discard(key, extra_keys.iter().copied()))
        {
            return Ok(());
        }
        let row = SortedRow {
            key,
            extra_keys,
            values,
            sequence,
            descending: self.descending,
            extra_descending: self.extra_descending.clone(),
        };
        let bytes = retained_row_size(&row)?;
        if self.requested > MAX_SORTED_ROWS && !self.rows.is_empty()
            && self.retained_bytes.checked_add(bytes)
                .is_none_or(|total| total > MAX_SORT_BUFFER_BYTES) {
            self.flush_run()?;
        }
        if self.rows.len() < retained {
            self.retained_bytes = updated_sort_bytes(self.retained_bytes, 0, bytes)?;
            self.rows.push(row);
        } else if self.requested <= MAX_SORTED_ROWS
            && self.rows.peek().is_some_and(|worst| row < *worst)
        {
            // Preserve the exact live budget when replacing the losing row.
            let discarded = retained_row_size(self.rows.peek().expect("nonempty sort heap"))?;
            self.retained_bytes = updated_sort_bytes(self.retained_bytes, discarded, bytes)?;
            *self.rows.peek_mut().expect("nonempty sort heap") = row;
        }
        Ok(())
    }

    pub(super) fn write<W: Write>(mut self, output: &mut W) -> Result<(), ExecutionError> {
        if !self.run_paths.is_empty() {
            self.flush_run()?;
            while self.run_paths.len() > MERGE_FAN_IN {
                let previous = std::mem::take(&mut self.run_paths);
                let mut reduced = Vec::new();
                for group in previous.chunks(MERGE_FAN_IN) {
                    self.preflight_merge_run(group)?;
                    let path = self.next_path()?;
                    let mut writer = BufWriter::with_capacity(64 * 1024, OpenOptions::new()
                        .create_new(true).write(true).open(&path)?);
                    let mut row_scratch = Vec::new();
                    merge_runs(group, self.descending, &self.extra_descending, |row| {
                        write_sorted_row_reusing(&mut writer, row, &mut row_scratch)?;
                        Ok(true)
                    })?;
                    writer.flush()?;
                    self.charge_temp(&path)?;
                    reduced.push(path);
                    for old in group {
                        self.delete_run(old)?;
                    }
                }
                self.run_paths = reduced;
            }
            let mut scanned = 0usize;
            let mut returned = 0usize;
            let limit = self.requested.saturating_sub(self.offset);
            merge_runs(&self.run_paths, self.descending, &self.extra_descending, |row| {
                if scanned >= self.requested { return Ok(false); }
                if scanned >= self.offset && returned < limit {
                    for (index, value) in row.values.iter().enumerate() {
                        if index != 0 { write!(output, "\t")?; }
                        write!(output, "{value}")?;
                    }
                    writeln!(output)?;
                    returned += 1;
                }
                scanned = scanned.checked_add(1)
                    .ok_or_else(|| invalid("external sort page count overflow"))?;
                Ok(scanned < self.requested)
            })?;
            return Ok(());
        }
        for row in self.rows.into_sorted_vec().into_iter().skip(self.offset) {
            for (index, value) in row.values.iter().enumerate() {
                if index != 0 {
                    write!(output, "\t")?;
                }
                write!(output, "{value}")?;
            }
            writeln!(output)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod serialization_tests {
    use super::*;

    #[test]
    fn numeric_records_keep_the_exact_external_sort_wire_format() {
        for number in [
            Number::Unsigned(u128::MAX),
            Number::Signed(-123),
            Number::Float(-0.0),
        ] {
            let mut bytes = Vec::new();
            write_number(&mut bytes, number).unwrap();
            assert_eq!(bytes.len(), 17);
            let mut input = bytes.as_slice();
            let recovered = read_number(&mut input).unwrap();
            match (number, recovered) {
                (Number::Unsigned(a), Number::Unsigned(b)) => assert_eq!(a, b),
                (Number::Signed(a), Number::Signed(b)) => assert_eq!(a, b),
                (Number::Float(a), Number::Float(b)) => assert_eq!(a.to_bits(), b.to_bits()),
                _ => panic!("numeric record changed type"),
            }
            assert!(input.is_empty());
        }
    }
}

#[cfg(test)]
#[path = "sort/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sort/spill_live_bytes_accounting_tests.rs"]
mod spill_live_bytes_accounting_tests;

#[cfg(test)]
#[path = "sort/preflight_external_merge_budget_tests.rs"]
mod preflight_external_merge_budget_tests;

#[cfg(test)]
#[path = "sort/sort_memory_budget_tests.rs"]
mod sort_memory_budget_tests;

#[cfg(test)]
mod external_sort_run_header_eof_tests {
    use super::*;

    #[test]
    fn empty_run_is_eof_but_partial_header_is_corruption() {
        let root = std::env::temp_dir().join(format!("rusthouse-sort-header-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("run.bin");
        let directions: Arc<[bool]> = Arc::from([]);
        fs::write(&path, []).unwrap();
        let mut reader = BufReader::new(File::open(&path).unwrap());
        assert!(read_sorted_row(&mut reader, false, &directions).unwrap().is_none());
        fs::write(&path, [42u8]).unwrap();
        let mut reader = BufReader::new(File::open(&path).unwrap());
        assert!(read_sorted_row(&mut reader, false, &directions).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod numeric_spill_preflight_tests {
    use super::*;

    #[test]
    fn checks_exact_serialized_size_before_allocating_scratch() {
        let mut rows = SortedRows::new(None, false);
        rows.set_extra_directions([false, true]);
        rows.push_with_extra(
            Number::Unsigned(1),
            vec![Number::Unsigned(2), Number::Signed(-3)],
            vec![Number::Unsigned(4)],
        ).unwrap();
        rows.temp_bytes = MAX_TEMP_SORT_BYTES - 83;
        assert!(rows.flush_run().is_err());
        assert!(rows.scratch.is_none());
        assert_eq!(rows.rows.len(), 1);

        rows.temp_bytes = 0;
        rows.flush_run().unwrap();
        let bytes = fs::metadata(&rows.run_paths[0]).unwrap().len();
        assert_eq!(bytes, 16 + 17 * 4);
        assert_eq!(rows.temp_bytes, bytes);
    }

    #[test]
    fn empty_run_needs_no_scratch_even_at_quota() {
        let mut rows = SortedRows::new(None, false);
        rows.temp_bytes = MAX_TEMP_SORT_BYTES;
        rows.flush_run().unwrap();
        assert!(rows.scratch.is_none());
    }
}

#[cfg(test)]
mod batched_sort_read_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn buffered_multi_key_rows_match_existing_wire_writer() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-batched-sort-read-{}-{}.bin",
            std::process::id(), NEXT.fetch_add(1, AtomicOrdering::Relaxed)
        ));
        let directions: Arc<[bool]> = Arc::from([true, false]);
        let rows = [
            SortedRow {
                key: Number::Unsigned(u128::MAX),
                extra_keys: vec![Number::Signed(i128::MIN), Number::Float(-0.0)],
                values: vec![Number::Unsigned(42), Number::Signed(-17)],
                sequence: 7, descending: true, extra_descending: directions.clone(),
            },
            SortedRow {
                key: Number::Float(f64::INFINITY),
                extra_keys: vec![Number::Signed(1024), Number::Float(2.5)],
                values: vec![Number::Float(0.125), Number::Unsigned(0)],
                sequence: 8, descending: true, extra_descending: directions.clone(),
            },
        ];
        let mut encoded = Vec::new();
        for row in &rows {
            write_sorted_row(&mut encoded, row).unwrap();
        }
        std::fs::write(&path, encoded).unwrap();
        let mut source = BufReader::with_capacity(64 * 1024, File::open(&path).unwrap());
        let mut scratch = Vec::new();
        for expected in &rows {
            let actual = read_sorted_row_reusing(&mut source, true, &directions, &mut scratch)
                .unwrap().unwrap();
            assert_eq!(actual.sequence, expected.sequence);
            assert_eq!(actual.key.to_string(), expected.key.to_string());
            assert_eq!(actual.extra_keys.iter().map(ToString::to_string).collect::<Vec<_>>(),
                       expected.extra_keys.iter().map(ToString::to_string).collect::<Vec<_>>());
            assert_eq!(actual.values.iter().map(ToString::to_string).collect::<Vec<_>>(),
                       expected.values.iter().map(ToString::to_string).collect::<Vec<_>>());
            assert_eq!(scratch.len(), (1 + 2 + 2) * 17);
        }
        assert!(read_sorted_row_reusing(&mut source, true, &directions, &mut scratch)
            .unwrap().is_none());
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod buffered_sort_record_tests {
    use super::*;

    #[test]
    fn reusable_row_buffer_matches_legacy_field_encoding() {
        let directions: Arc<[bool]> = Arc::from([true, false]);
        let rows = [
            (Number::Signed(-9), vec![Number::Unsigned(42), Number::Float(-0.0)],
                vec![Number::Unsigned(u128::MAX), Number::Float(f64::NAN)]),
            (Number::Float(3.5), vec![Number::Signed(i128::MIN), Number::Float(f64::INFINITY)],
                vec![]),
        ];
        let mut scratch = Vec::new();
        let mut complete = Vec::new();
        let mut reference = Vec::new();
        for (position, (key, extra_keys, values)) in rows.into_iter().enumerate() {
            let row = SortedRow {
                key, extra_keys, values, sequence: position,
                descending: false, extra_descending: directions.clone(),
            };
            write_sorted_row_reusing(&mut complete, &row, &mut scratch).unwrap();
            reference.extend_from_slice(&(position as u64).to_le_bytes());
            reference.extend_from_slice(&(row.extra_keys.len() as u32).to_le_bytes());
            reference.extend_from_slice(&(row.values.len() as u32).to_le_bytes());
            write_number(&mut reference, row.key).unwrap();
            for &number in &row.extra_keys { write_number(&mut reference, number).unwrap(); }
            for &number in &row.values { write_number(&mut reference, number).unwrap(); }
        }
        assert_eq!(complete, reference);
    }
}


#[cfg(test)]
mod external_run_order_validation_tests {
    use super::*;

    fn run(scratch: &SortScratch, name: &str, keys: &[u128], descending: bool)
        -> PathBuf {
        let path = scratch.0.join(name);
        let mut writer = BufWriter::new(File::create(&path).unwrap());
        let directions: Arc<[bool]> = Arc::from([]);
        for (sequence, &key) in keys.iter().enumerate() {
            write_sorted_row(&mut writer, &SortedRow {
                key: Number::Unsigned(key),
                extra_keys: Vec::new(),
                values: vec![Number::Unsigned(key)],
                sequence,
                descending,
                extra_descending: directions.clone(),
            }).unwrap();
        }
        writer.flush().unwrap();
        path
    }

    #[test]
    fn validates_order_in_single_run_and_multi_run_merges() {
        let scratch = SortScratch::new().unwrap();
        let directions: Arc<[bool]> = Arc::from([]);
        let invalid = run(&scratch, "bad", &[1, 9, 4], false);
        let valid = run(&scratch, "good", &[2, 3, 5], false);
        for paths in [vec![invalid.clone()], vec![invalid.clone(), valid.clone()]] {
            let error = merge_runs(&paths, false, &directions, |_| Ok(true)).unwrap_err();
            assert!(error.to_string().contains("not sorted"), "{error}");
        }
    }

    #[test]
    fn validates_descending_order_and_preserves_early_page_stop() {
        let scratch = SortScratch::new().unwrap();
        let directions: Arc<[bool]> = Arc::from([]);
        let invalid = run(&scratch, "bad-desc", &[9, 4, 8], true);
        assert!(merge_runs(&[invalid.clone()], true, &directions, |_| Ok(true)).is_err());
        // Do not inspect a later corrupt or out-of-order row after LIMIT.
        let mut seen = 0;
        merge_runs(&[invalid], true, &directions, |_| {
            seen += 1;
            Ok(false)
        }).unwrap();
        assert_eq!(seen, 1);
    }
}
