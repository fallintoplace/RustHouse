//! Own the typed multi-part cursor and its prefetch worker in one focused module.

use super::*;

struct PartPrefetchWorker {
    queue: Option<std::sync::mpsc::SyncSender<PathBuf>>,
    results: std::sync::mpsc::Receiver<Result<TypedPartReader, PartError>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PartPrefetchWorker {
    fn new(columns: std::sync::Arc<[String]>) -> Option<Self> {
        let (queue, requests) = std::sync::mpsc::sync_channel::<PathBuf>(1);
        let (responses, results) =
            std::sync::mpsc::sync_channel::<Result<TypedPartReader, PartError>>(1);
        let thread = std::thread::Builder::new()
            .spawn(move || {
                while let Ok(path) = requests.recv() {
                    if responses
                        .send(TypedPartReader::open(&path, &columns))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self {
            queue: Some(queue),
            results,
            thread: Some(thread),
        })
    }
}

impl Drop for PartPrefetchWorker {
    fn drop(&mut self) {
        // Disconnect requests before joining, including on LIMIT and error.
        self.queue.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Read typed rows from one part or a frozen multi-part snapshot.
/// Only one part's file streams are held open at a time.
pub struct TypedQueryReader {
    parts: Vec<PathBuf>,
    columns: std::sync::Arc<[String]>,
    next_part: usize,
    current: Option<TypedPartReader>,
    // At most one part-open result is staged ahead, never decoded batches.
    prefetched: bool,
    worker: Option<PartPrefetchWorker>,
    #[cfg(test)]
    prefetch_worker_starts: usize,
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
            columns: std::sync::Arc::from(columns.to_vec()),
            next_part: 0,
            current: None,
            prefetched: false,
            worker: None,
            #[cfg(test)]
            prefetch_worker_starts: 0,
        })
    }

    // Schedule one validated part open on the persistent worker. When a
    // receiver fails, fall back to synchronous opens without losing a part.
    fn prefetch_next(&mut self) {
        // A two or three part snapshot usually completes its I/O faster
        // than a fresh channel-backed thread can start and join.
        if self.parts.len() < 4 {
            return;
        }
        if self.prefetched || self.next_part >= self.parts.len() {
            return;
        }
        if self.worker.is_none() {
            self.worker = PartPrefetchWorker::new(self.columns.clone());
            if self.worker.is_none() {
                // Keep the synchronous scan usable under thread exhaustion.
                return;
            }
            #[cfg(test)]
            {
                self.prefetch_worker_starts += 1;
            }
        }
        let path = self.parts[self.next_part].clone();
        let queued = self
            .worker
            .as_ref()
            .and_then(|worker| worker.queue.as_ref())
            .is_some_and(|queue| queue.send(path).is_ok());
        if queued {
            self.next_part += 1;
            self.prefetched = true;
        } else {
            self.worker.take();
        }
    }

    fn advance_part(&mut self) -> Result<bool, PartError> {
        let reader = if self.prefetched {
            self.prefetched = false;
            self.worker
                .as_ref()
                .ok_or_else(|| PartError::InvalidScan("missing part prefetch worker".into()))?
                .results
                .recv()
                .map_err(|_| {
                    PartError::InvalidScan("typed snapshot part opener stopped".into())
                })??
        } else if self.next_part < self.parts.len() {
            let reader = TypedPartReader::open(&self.parts[self.next_part], &self.columns)?;
            self.next_part += 1;
            reader
        } else {
            return Ok(false);
        };
        self.current = Some(reader);
        self.prefetch_next();
        Ok(true)
    }

    /// Read NULL maps across a multi-part snapshot without decoding values.
    pub fn next_null_batch(&mut self) -> Result<Option<Vec<u8>>, PartError> {
        if self.columns.len() != 1 {
            return Err(PartError::InvalidScan(
                "null-map scan requires exactly one column".into(),
            ));
        }
        if self.current.is_some() {
            self.prefetch_next();
        }
        loop {
            if let Some(reader) = self.current.as_mut() {
                if let Some(nulls) = reader.next_null_batch()? {
                    return Ok(Some(nulls));
                }
                self.current = None;
            }
            if !self.advance_part()? {
                return Ok(None);
            }
        }
    }

    /// Skip whole parts and adaptive marks for an unfiltered, unordered
    /// OFFSET query. Only valid on a fresh snapshot reader.
    pub fn skip_prefix_rows(&mut self, max_rows: usize) -> Result<usize, PartError> {
        if self.next_part != 0 || self.current.is_some() {
            return Err(PartError::InvalidScan(
                "typed OFFSET mark skipping requires a fresh reader".into(),
            ));
        }
        let mut skipped = 0usize;
        while skipped < max_rows && self.next_part < self.parts.len() {
            let part = &self.parts[self.next_part];
            let rows = read_part_row_count(part)?;
            if let Ok(rows) = usize::try_from(rows) {
                if rows <= max_rows - skipped {
                    skipped += rows;
                    self.next_part += 1;
                    continue;
                }
            }
            // The final partially skipped part still validates aligned marks.
            let mut reader = TypedPartReader::open(part, &self.columns)?;
            self.next_part += 1;
            skipped += reader.skip_complete_marks(max_rows - skipped);
            if reader.next_mark != reader.mark_count() {
                self.current = Some(reader);
                break;
            }
        }
        Ok(skipped)
    }

    pub fn next_batch(&mut self) -> Result<Option<Vec<TypedColumn>>, PartError> {
        // A preceding OFFSET skip may leave a partially consumed part open.
        if self.current.is_some() {
            self.prefetch_next();
        }
        loop {
            if let Some(reader) = self.current.as_mut() {
                if let Some(batch) = reader.next_batch()? {
                    return Ok(Some(batch));
                }
                self.current = None;
            }
            if !self.advance_part()? {
                return Ok(None);
            }
        }
    }
}

impl Drop for TypedQueryReader {
    fn drop(&mut self) {
        // Disconnect and join the persistent opener before releasing the scan.
        self.worker.take();
    }
}
