//! Exact high-cardinality GROUP BY counts using sorted, reduced disk runs.
use super::ExecutionError;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(not(test))]
const RUN_KEYS: usize = 1_048_576;
#[cfg(test)]
const RUN_KEYS: usize = 2048;
const FAN_IN: usize = 32;
const MAX_DISK_BYTES: u64 = 32 * 1024 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);

fn invalid(message: impl Into<String>) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}

struct Scratch(PathBuf);
impl Scratch {
    fn create() -> Result<Self, ExecutionError> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|cause| invalid(cause.to_string()))?.as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rusthouse-external-grouped-{}-{stamp}-{}",
            std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
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
impl Drop for Scratch {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

// Temporary sorted runs are private to a query, so use compact canonical
// unsigned varints instead of 16-byte fixed-size key/count pairs. This cuts
// disk bandwidth substantially on high-cardinality 32-bit dimensions.
fn read_varint(input: &mut impl BufRead, eof_allowed: bool) -> Result<Option<u64>, ExecutionError> {
    let mut result = 0_u64;
    let mut index = 0;
    while index < 10 {
        let block = input.fill_buf()?;
        if block.is_empty() {
            if index == 0 && eof_allowed { return Ok(None); }
            return Err(invalid("truncated GROUP BY spill varint"));
        }
        let mut used = 0;
        while used < block.len() && index < 10 {
            let byte = block[used];
            used += 1;
            let value = byte & 0x7f;
            if index == 9 && value > 1 {
                return Err(invalid("GROUP BY spill varint overflow"));
            }
            result |= u64::from(value) << (index * 7);
            if byte & 0x80 == 0 {
                if index > 0 && value == 0 {
                    return Err(invalid("noncanonical GROUP BY spill varint"));
                }
                input.consume(used);
                return Ok(Some(result));
            }
            index += 1;
        }
        input.consume(used);
    }
    Err(invalid("GROUP BY spill varint too long"))
}

fn write_varint(output: &mut impl Write, mut value: u64) -> Result<(), ExecutionError> {
    while value >= 128 {
        output.write_all(&[((value as u8) & 0x7f) | 0x80])?;
        value >>= 7;
    }
    output.write_all(&[value as u8])?;
    Ok(())
}

fn read_group(reader: &mut BufReader<File>)
    -> Result<Option<(u64, u64)>, ExecutionError> {
    let Some(key) = read_varint(reader, true)? else { return Ok(None); };
    let count = read_varint(reader, false)?
        .ok_or_else(|| invalid("truncated GROUP BY spill count"))?;
    if count == 0 { return Err(invalid("invalid empty grouped spill entry")); }
    Ok(Some((key, count)))
}

fn write_group(output: &mut impl Write, key: u64, count: u64)
    -> Result<(), ExecutionError> {
    write_varint(output, key)?;
    write_varint(output, count)?;
    Ok(())
}

fn merge_until(
    files: &[PathBuf],
    mut emit: impl FnMut(u64, u64) -> Result<bool, ExecutionError>,
) -> Result<(), ExecutionError> {
    let mut streams = files.iter().map(File::open)
        .map(|reader| reader.map(BufReader::new))
        .collect::<Result<Vec<_>, _>>()?;
    let mut heap = BinaryHeap::<Reverse<(u64, usize, u64)>>::new();
    for (index, stream) in streams.iter_mut().enumerate() {
        if let Some((key, count)) = read_group(stream)? {
            heap.push(Reverse((key, index, count)));
        }
    }
    let mut current: Option<(u64, u64)> = None;
    while let Some(Reverse((key, index, count))) = heap.pop() {
        match current.as_mut() {
            Some((last, total)) if *last == key => {
                *total = total.checked_add(count)
                    .ok_or_else(|| invalid("GROUP BY count overflow"))?;
            }
            _ => {
                if let Some((last, total)) = current.take() {
                    if !emit(last, total)? { return Ok(()); }
                }
                current = Some((key, count));
            }
        }
        if let Some((next, count)) = read_group(&mut streams[index])? {
            heap.push(Reverse((next, index, count)));
        }
    }
    if let Some((last, count)) = current { let _ = emit(last, count)?; }
    Ok(())
}

/// Full-run merge for intermediate compaction: every key must be written.
fn merge(
    files: &[PathBuf],
    mut emit: impl FnMut(u64, u64) -> Result<(), ExecutionError>,
) -> Result<(), ExecutionError> {
    merge_until(files, |key, count| { emit(key, count)?; Ok(true) })
}

/// Stable eight-pass LSD radix sort for large encoded UInt64 group keys.
/// Each key requires bounded scratch space and no comparison-based branches.
/// Small runs keep the standard library sort to avoid histogram overhead.
fn sort_run_keys(keys: &mut Vec<u64>) {
    if keys.len() < 16_384 {
        keys.sort_unstable();
        return;
    }
    let mut scratch = vec![0u64; keys.len()];
    for shift in (0..64).step_by(8) {
        let mut offsets = [0usize; 256];
        for &key in keys.iter() {
            offsets[((key >> shift) & 255) as usize] += 1;
        }
        // Whole-byte uniformity is common on narrow dimensions, so do not
        // copy the input for passes that cannot change its ordering.
        if offsets.iter().any(|&count| count == keys.len()) {
            continue;
        }
        let mut begin = 0usize;
        for count in &mut offsets {
            let next = begin + *count;
            *count = begin;
            begin = next;
        }
        for &key in keys.iter() {
            let bucket = ((key >> shift) & 255) as usize;
            scratch[offsets[bucket]] = key;
            offsets[bucket] += 1;
        }
        std::mem::swap(keys, &mut scratch);
    }
}

pub(super) struct ExternalGroupCount64 {
    scratch: Option<Scratch>,
    keys: Vec<u64>,
    runs: Vec<PathBuf>,
    next_run: usize,
    disk_bytes: u64,
    signed: bool,
    descending: bool,
}

impl ExternalGroupCount64 {
    pub(super) fn new(signed: bool, descending: bool) -> Self {
        Self {
            scratch: None, keys: Vec::with_capacity(RUN_KEYS.min(8192)),
            runs: Vec::new(), next_run: 0, disk_bytes: 0,
            signed, descending,
        }
    }
    pub(super) fn insert(&mut self, value: u64) -> Result<(), ExecutionError> {
        let sortable = if self.signed { value ^ (1 << 63) } else { value };
        self.keys.push(if self.descending { !sortable } else { sortable });
        if self.keys.len() == RUN_KEYS { self.flush()?; }
        Ok(())
    }
    fn next_path(&mut self) -> Result<PathBuf, ExecutionError> {
        if self.scratch.is_none() { self.scratch = Some(Scratch::create()?); }
        let index = self.next_run;
        self.next_run = self.next_run.checked_add(1)
            .ok_or_else(|| invalid("too many GROUP BY spill runs"))?;
        Ok(self.scratch.as_ref().unwrap().0.join(format!("run-{index:08}.bin")))
    }
    fn reserve_disk(&mut self, path: &Path) -> Result<(), ExecutionError> {
        self.disk_bytes = self.disk_bytes.checked_add(fs::metadata(path)?.len())
            .ok_or_else(|| invalid("GROUP BY spill byte overflow"))?;
        if self.disk_bytes > MAX_DISK_BYTES {
            return Err(invalid("GROUP BY exceeded the 32 GiB temporary disk budget"));
        }
        Ok(())
    }
    // Reclaim the quota only after the previous run has actually been
    // removed. Counting obsolete files after each fan-in pass can reject
    // queries whose current temporary disk footprint is well below budget.
    fn remove_run(&mut self, path: &Path) -> Result<(), ExecutionError> {
        let bytes = fs::metadata(path)?.len();
        fs::remove_file(path)?;
        self.disk_bytes = self.disk_bytes.checked_sub(bytes)
            .ok_or_else(|| invalid("GROUP BY spill accounting underflow"))?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), ExecutionError> {
        if self.keys.is_empty() { return Ok(()); }
        sort_run_keys(&mut self.keys);
        let path = self.next_path()?;
        let mut stream = BufWriter::new(OpenOptions::new()
            .write(true).create_new(true).open(&path)?);
        let mut previous = None;
        let mut count = 0u64;
        for &key in &self.keys {
            if previous != Some(key) {
                if let Some(last) = previous { write_group(&mut stream, last, count)?; }
                previous = Some(key);
                count = 1;
            } else {
                count = count.checked_add(1)
                    .ok_or_else(|| invalid("GROUP BY count overflow"))?;
            }
        }
        if let Some(last) = previous { write_group(&mut stream, last, count)?; }
        stream.flush()?;
        self.reserve_disk(&path)?;
        self.runs.push(path);
        self.keys.clear();
        Ok(())
    }
    fn decode_key(&self, encoded: u64) -> u64 {
        let key = if self.descending { !encoded } else { encoded };
        if self.signed { key ^ (1 << 63) } else { key }
    }
    fn compact_runs(&mut self) -> Result<(), ExecutionError> {
            while self.runs.len() > FAN_IN {
                let previous = std::mem::take(&mut self.runs);
                let mut reduced = Vec::new();
                for chunk in previous.chunks(FAN_IN) {
                    let path = self.next_path()?;
                    let mut writer = BufWriter::new(OpenOptions::new()
                        .create_new(true).write(true).open(&path)?);
                    merge(chunk, |key, count| write_group(&mut writer, key, count))?;
                    writer.flush()?;
                    self.reserve_disk(&path)?;
                    reduced.push(path);
                    for old in chunk { self.remove_run(old)?; }
                }
                self.runs = reduced;
            }
        Ok(())
    }

    /// Stop the final k-way merge when the client has received its page.
    /// Intermediate fan-in runs are always fully reduced before this step.
    pub(super) fn finish_until(
        mut self, mut emit: impl FnMut(u64, u64) -> Result<bool, ExecutionError>,
    ) -> Result<(), ExecutionError> {
        self.flush()?;
        self.compact_runs()?;
        let signed = self.signed;
        let descending = self.descending;
        merge_until(&self.runs, |encoded, count| {
            let key = if descending { !encoded } else { encoded };
            let key = if signed { key ^ (1 << 63) } else { key };
            emit(key, count)
        })
    }

    pub(super) fn finish(
        self, mut emit: impl FnMut(u64, u64) -> Result<(), ExecutionError>,
    ) -> Result<(), ExecutionError> {
        self.finish_until(|key, count| { emit(key, count)?; Ok(true) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn grouped_spill_directory_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::create().unwrap();
        assert_eq!(fs::metadata(&scratch.0).unwrap().permissions().mode() & 0o077, 0);
    }

    #[test]
    fn spills_and_merges_more_than_thirty_two_sorted_group_runs() {
        let mut group = ExternalGroupCount64::new(false, false);
        for id in (0..(RUN_KEYS * 35)).rev() {
            group.insert((id % 50_000) as u64).unwrap();
        }
        let mut values = Vec::new();
        group.finish(|key, count| {
            values.push((key, count));
            Ok(())
        }).unwrap();
        assert_eq!(values.len(), 50_000);
        assert!(values.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(values.iter().map(|(_, count)| *count).sum::<u64>(),
            (RUN_KEYS * 35) as u64);
    }

    #[test]
    fn compact_spills_reduce_io_for_billion_scale_dimension_ids() {
        let mut groups = ExternalGroupCount64::new(false, false);
        for id in 0..RUN_KEYS {
            groups.insert(id as u64).unwrap();
        }
        assert_eq!(groups.runs.len(), 1);
        let bytes = fs::metadata(&groups.runs[0]).unwrap().len();
        // A fixed-width run would need 16 bytes for every unique group.
        assert!(bytes < (RUN_KEYS as u64) * 8, "{bytes} bytes");
        let mut total = 0_u64;
        groups.finish(|_, count| { total += count; Ok(()) }).unwrap();
        assert_eq!(total, RUN_KEYS as u64);
    }


    #[test]
    fn buffered_varints_survive_single_byte_buffer_boundaries() {
        let mut encoded = Vec::new();
        for value in [0, 127, 128, 16_384, u64::MAX] {
            write_varint(&mut encoded, value).unwrap();
        }
        let mut reader = BufReader::with_capacity(1, encoded.as_slice());
        for expected in [0, 127, 128, 16_384, u64::MAX] {
            assert_eq!(read_varint(&mut reader, false).unwrap(), Some(expected));
        }
        assert_eq!(read_varint(&mut reader, true).unwrap(), None);
    }

    #[test]
    fn spill_varints_reject_truncation_and_overflow() {
        let mut truncated = &[0x80_u8][..];
        assert!(read_varint(&mut truncated, true).is_err());
        let mut excessive = &[0xff_u8; 11][..];
        assert!(read_varint(&mut excessive, true).is_err());
        let mut out = Vec::new();
        write_varint(&mut out, u64::MAX).unwrap();
        assert_eq!(read_varint(&mut out.as_slice(), false).unwrap(), Some(u64::MAX));
    }

    #[test]
    fn multi_pass_spill_budget_counts_only_existing_runs() {
        let mut group = ExternalGroupCount64::new(false, false);
        for id in 0..(RUN_KEYS * (FAN_IN + 3)) {
            group.insert(id as u64).unwrap();
        }
        assert!(group.runs.len() > FAN_IN);
        group.compact_runs().unwrap();
        let actual: u64 = group.runs.iter()
            .map(|path| fs::metadata(path).unwrap().len()).sum();
        assert_eq!(group.disk_bytes, actual);
        assert!(group.runs.len() <= FAN_IN);
        let mut seen = 0_u64;
        group.finish(|_, count| { seen += count; Ok(()) }).unwrap();
        assert_eq!(seen, (RUN_KEYS * (FAN_IN + 3)) as u64);
    }

    #[test]
    fn paged_final_merge_stops_after_requested_groups() {
        let mut group = ExternalGroupCount64::new(false, false);
        for index in (0..(RUN_KEYS * (FAN_IN + 2))).rev() {
            group.insert((index % 50_000) as u64).unwrap();
        }
        let mut seen = 0usize;
        let mut selected = Vec::new();
        group.finish_until(|key, count| {
            if seen >= 100 && selected.len() < 7 { selected.push((key, count)); }
            seen += 1;
            Ok(selected.len() < 7)
        }).unwrap();
        assert_eq!(seen, 107);
        assert_eq!(selected.iter().map(|row| row.0).collect::<Vec<_>>(),
            (100u64..107).collect::<Vec<_>>());
        assert!(selected.iter().all(|row| row.1 > 0));
    }

    #[test]
    fn early_stop_keeps_distinct_keys_fully_merged() {
        let mut group = ExternalGroupCount64::new(true, true);
        for value in [-1_i64, 1, -1, 3, 3, 3, 0] {
            group.insert(value as u64).unwrap();
        }
        let mut result = Vec::new();
        group.finish_until(|key, count| {
            result.push((key as i64, count));
            Ok(result.len() < 2)
        }).unwrap();
        assert_eq!(result, [(3, 3), (1, 1)]);
    }

    #[test]
    fn radix_sort_matches_full_u64_reference_on_large_random_runs() {
        let mut keys = Vec::with_capacity(100_003);
        let mut seed = 0x9e3779b97f4a7c15_u64;
        for index in 0..100_003_u64 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            keys.push(seed ^ index.rotate_left(23));
        }
        let mut expected = keys.clone();
        expected.sort_unstable();
        sort_run_keys(&mut keys);
        assert_eq!(keys, expected);
    }

    #[test]
    fn radix_sort_preserves_duplicates_and_high_sign_bits() {
        let mut keys = Vec::new();
        for index in (0..50_000_u64).rev() {
            keys.push(index.wrapping_mul(u64::MAX / 65535));
            keys.push(u64::MAX);
            keys.push(1_u64 << 63);
        }
        let mut expected = keys.clone();
        expected.sort_unstable();
        sort_run_keys(&mut keys);
        assert_eq!(keys, expected);
    }

    #[test]
    fn signed_descending_sort_preserves_negative_keys() {
        let mut groups = ExternalGroupCount64::new(true, true);
        for number in [-1i64, -5, 1, 1, 0] { groups.insert(number as u64).unwrap(); }
        let mut rows = Vec::new();
        groups.finish(|key, count| { rows.push((key as i64, count)); Ok(()) }).unwrap();
        assert_eq!(rows, vec![(1, 2), (0, 1), (-1, 1), (-5, 1)]);
    }
}
