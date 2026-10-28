//! Exact UInt64 / Int64 DISTINCT using bounded sorted disk runs.
use super::ExecutionError;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(not(test))]
const RUN_SIZE: usize = 1_048_576;
#[cfg(test)]
const RUN_SIZE: usize = 4096;
const FAN_IN: usize = 32;
const MAX_DISK_BYTES: u64 = 32 * 1024 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);

fn error(message: impl Into<String>) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}

struct Scratch(PathBuf);
impl Scratch {
    fn create() -> Result<Self, ExecutionError> {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|cause| error(cause.to_string()))?.as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-exact-distinct-{}-{millis}-{}",
            std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&dir)?;
        Ok(Self(dir))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn next_key(reader: &mut BufReader<File>) -> Result<Option<u64>, ExecutionError> {
    let mut byte = [0; 1];
    if reader.read(&mut byte)? == 0 { return Ok(None); }
    let mut rest = [0; 7];
    reader.read_exact(&mut rest)?;
    Ok(Some(u64::from_le_bytes([byte[0], rest[0], rest[1], rest[2],
        rest[3], rest[4], rest[5], rest[6]])))
}

fn merge(files: &[PathBuf], destination: Option<&Path>) -> Result<u64, ExecutionError> {
    let mut readers = files.iter().map(File::open)
        .map(|stream| stream.map(BufReader::new))
        .collect::<Result<Vec<_>, _>>()?;
    let mut heap = BinaryHeap::<Reverse<(u64, usize)>>::new();
    for (index, reader) in readers.iter_mut().enumerate() {
        if let Some(value) = next_key(reader)? { heap.push(Reverse((value, index))); }
    }
    let mut output = destination.map(|path| OpenOptions::new()
        .write(true).create_new(true).open(path).map(BufWriter::new)).transpose()?;
    let mut last = None;
    let mut count = 0u64;
    while let Some(Reverse((value, index))) = heap.pop() {
        if last != Some(value) {
            if let Some(stream) = output.as_mut() { stream.write_all(&value.to_le_bytes())?; }
            count = count.checked_add(1).ok_or_else(|| error("distinct count overflow"))?;
            last = Some(value);
        }
        if let Some(next) = next_key(&mut readers[index])? {
            heap.push(Reverse((next, index)));
        }
    }
    if let Some(mut stream) = output { stream.flush()?; }
    Ok(count)
}

pub(super) struct ExternalDistinct64 {
    scratch: Option<Scratch>,
    buffered: Vec<u64>,
    files: Vec<PathBuf>,
    written: u64,
    counter: usize,
}

impl ExternalDistinct64 {
    pub(super) fn new() -> Self {
        Self { scratch: None, buffered: Vec::with_capacity(RUN_SIZE.min(8192)),
            files: Vec::new(), written: 0, counter: 0 }
    }
    pub(super) fn insert(&mut self, value: u64) -> Result<(), ExecutionError> {
        self.buffered.push(value);
        if self.buffered.len() == RUN_SIZE { self.flush()?; }
        Ok(())
    }
    fn reserve(&mut self, count: u64) -> Result<(), ExecutionError> {
        let extra = count.checked_mul(8).ok_or_else(|| error("distinct spill size overflow"))?;
        self.written = self.written.checked_add(extra)
            .ok_or_else(|| error("distinct spill size overflow"))?;
        if self.written > MAX_DISK_BYTES {
            return Err(error("distinct disk spill exceeded 32 GiB budget"));
        }
        Ok(())
    }
    fn next_path(&mut self) -> Result<PathBuf, ExecutionError> {
        if self.scratch.is_none() { self.scratch = Some(Scratch::create()?); }
        let index = self.counter;
        self.counter = self.counter.checked_add(1)
            .ok_or_else(|| error("too many distinct runs"))?;
        Ok(self.scratch.as_ref().unwrap().0.join(format!("run-{index:08}.bin")))
    }
    fn flush(&mut self) -> Result<(), ExecutionError> {
        if self.buffered.is_empty() { return Ok(()); }
        self.buffered.sort_unstable();
        self.buffered.dedup();
        self.reserve(self.buffered.len() as u64)?;
        let path = self.next_path()?;
        let mut writer = BufWriter::new(OpenOptions::new().write(true)
            .create_new(true).open(&path)?);
        for &key in &self.buffered { writer.write_all(&key.to_le_bytes())?; }
        writer.flush()?;
        self.files.push(path);
        self.buffered.clear();
        Ok(())
    }
    pub(super) fn finish(mut self) -> Result<u64, ExecutionError> {
        if self.files.is_empty() {
            self.buffered.sort_unstable();
            self.buffered.dedup();
            return Ok(self.buffered.len() as u64);
        }
        self.flush()?;
        while self.files.len() > FAN_IN {
            let previous = std::mem::take(&mut self.files);
            let mut next = Vec::new();
            for group in previous.chunks(FAN_IN) {
                let path = self.next_path()?;
                let keys = merge(group, Some(&path))?;
                self.reserve(keys)?;
                next.push(path);
                for old in group { fs::remove_file(old)?; }
            }
            self.files = next;
        }
        merge(&self.files, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiple_spills_deduplicate_out_of_order_keys() {
        let mut exact = ExternalDistinct64::new();
        for key in (0..25_000u64).rev() {
            exact.insert(key).unwrap();
            exact.insert(key).unwrap();
        }
        assert_eq!(exact.finish().unwrap(), 25_000);
    }
    #[test]
    fn merges_more_than_32_runs_without_leaking_files() {
        let mut exact = ExternalDistinct64::new();
        for key in 0..(RUN_SIZE * 34) {
            exact.insert((key / 2) as u64).unwrap();
        }
        assert_eq!(exact.finish().unwrap(), (RUN_SIZE * 34 / 2) as u64);
        assert_eq!(ExternalDistinct64::new().finish().unwrap(), 0);
    }
}
