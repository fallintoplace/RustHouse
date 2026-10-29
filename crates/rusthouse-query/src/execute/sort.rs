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

fn write_number(writer: &mut impl Write, number: Number) -> Result<(), ExecutionError> {
    let (tag, payload) = match number {
        Number::Unsigned(v) => (0u8, v.to_le_bytes()),
        Number::Signed(v) => (1u8, v.to_le_bytes()),
        Number::Float(v) => (2u8, (v.to_bits() as u128).to_le_bytes()),
    };
    writer.write_all(&[tag])?;
    writer.write_all(&payload)?;
    Ok(())
}

fn read_number(reader: &mut impl Read) -> Result<Number, ExecutionError> {
    let mut encoded = [0u8; 17];
    reader.read_exact(&mut encoded)?;
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

fn write_sorted_row(writer: &mut impl Write, row: &SortedRow)
    -> Result<(), ExecutionError> {
    let index = u64::try_from(row.sequence).map_err(|_| invalid("sort sequence overflow"))?;
    let secondary = u32::try_from(row.extra_keys.len())
        .map_err(|_| invalid("too many ORDER BY keys"))?;
    let width = u32::try_from(row.values.len())
        .map_err(|_| invalid("too many sorted columns"))?;
    writer.write_all(&index.to_le_bytes())?;
    writer.write_all(&secondary.to_le_bytes())?;
    writer.write_all(&width.to_le_bytes())?;
    write_number(writer, row.key)?;
    for &key in &row.extra_keys { write_number(writer, key)?; }
    for &value in &row.values { write_number(writer, value)?; }
    Ok(())
}

fn read_sorted_row(
    reader: &mut BufReader<File>,
    descending: bool,
    directions: &Arc<[bool]>,
) -> Result<Option<SortedRow>, ExecutionError> {
    let mut sequence = [0u8; 8];
    if reader.read(&mut sequence[..1])? == 0 { return Ok(None); }
    reader.read_exact(&mut sequence[1..])?;
    let sequence = usize::try_from(u64::from_le_bytes(sequence))
        .map_err(|_| invalid("external sort sequence overflow"))?;
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    let extra = u32::from_le_bytes(bytes) as usize;
    reader.read_exact(&mut bytes)?;
    let width = u32::from_le_bytes(bytes) as usize;
    if extra != directions.len() || width > 128 {
        return Err(invalid("malformed external sort run width"));
    }
    let key = read_number(reader)?;
    let mut extra_keys = Vec::with_capacity(extra);
    for _ in 0..extra { extra_keys.push(read_number(reader)?); }
    let mut values = Vec::with_capacity(width);
    for _ in 0..width { values.push(read_number(reader)?); }
    Ok(Some(SortedRow {
        key, extra_keys, values, sequence,
        descending, extra_descending: directions.clone(),
    }))
}

fn merge_runs(
    inputs: &[PathBuf],
    descending: bool,
    directions: &Arc<[bool]>,
    mut output: impl FnMut(&SortedRow) -> Result<bool, ExecutionError>,
) -> Result<(), ExecutionError> {
    let mut streams = inputs.iter().map(File::open)
        .map(|source| source.map(|file| BufReader::with_capacity(64 * 1024, file)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut heap = BinaryHeap::<Reverse<(SortedRow, usize)>>::new();
    for (index, stream) in streams.iter_mut().enumerate() {
        if let Some(row) = read_sorted_row(stream, descending, directions)? {
            heap.push(Reverse((row, index)));
        }
    }
    while let Some(Reverse((row, index))) = heap.pop() {
        if !output(&row)? { break; }
        if let Some(next) = read_sorted_row(&mut streams[index], descending, directions)? {
            heap.push(Reverse((next, index)));
        }
    }
    Ok(())
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

    fn flush_run(&mut self) -> Result<(), ExecutionError> {
        if self.rows.is_empty() { return Ok(()); }
        let path = self.next_path()?;
        let mut writer = BufWriter::with_capacity(64 * 1024, OpenOptions::new()
            .create_new(true).write(true).open(&path)?);
        for row in std::mem::take(&mut self.rows).into_sorted_vec() {
            write_sorted_row(&mut writer, &row)?;
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
                    merge_runs(group, self.descending, &self.extra_descending, |row| {
                        write_sorted_row(&mut writer, row)?;
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
