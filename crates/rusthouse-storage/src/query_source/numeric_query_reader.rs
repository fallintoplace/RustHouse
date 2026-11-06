//! Keep snapshot discovery separate from the numeric multi-part reader lifecycle.

use super::*;

struct NumericPartPrefetchWorker {
    requests: Option<std::sync::mpsc::SyncSender<PathBuf>>,
    responses: std::sync::mpsc::Receiver<Result<NumericPartReader, PartError>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl NumericPartPrefetchWorker {
    fn new(columns: std::sync::Arc<[String]>) -> Option<Self> {
        let (requests, jobs) = std::sync::mpsc::sync_channel::<PathBuf>(1);
        let (answers, responses) =
            std::sync::mpsc::sync_channel::<Result<NumericPartReader, PartError>>(1);
        let thread = std::thread::Builder::new()
            .spawn(move || {
                while let Ok(path) = jobs.recv() {
                    if answers
                        .send(NumericPartReader::open(&path, &columns))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self {
            requests: Some(requests),
            responses,
            thread: Some(thread),
        })
    }
}

impl Drop for NumericPartPrefetchWorker {
    fn drop(&mut self) {
        // Disconnect requests first so a worker waiting for more jobs exits.
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Keep only one part's streams open at once; the executor sees a single
/// continuous sequence, so GROUP BY, ORDER BY, DISTINCT and LIMIT are global.
pub struct NumericQueryReader {
    parts: Vec<PathBuf>,
    columns: std::sync::Arc<[String]>,
    next_part: usize,
    current: Option<NumericPartReader>,
    // One prefetched part open, never prefetched decoded data batches.
    prefetched: bool,
    worker: Option<NumericPartPrefetchWorker>,
    #[cfg(test)]
    prefetch_worker_starts: usize,
}

impl NumericQueryReader {
    pub fn open(path: &Path, columns: &[String]) -> Result<Self, PartError> {
        let parts = query_parts(path)?;
        let metadata = inspect_parts_metadata(path, &parts)?;
        Self::from_validated_parts(parts, columns, &metadata)
    }

    /// Reuse metadata that was validated for this immutable snapshot by
    /// the query planner, rather than opening every part's metadata twice.
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
        for column in columns {
            if !metadata.columns.iter().any(|c: &Column| &c.name == column) {
                return Err(PartError::InvalidScan(format!("unknown column: {column}")));
            }
        }
        Ok(Self {
            parts,
            columns: std::sync::Arc::from(columns.to_owned()),
            next_part: 0,
            current: None,
            prefetched: false,
            worker: None,
            #[cfg(test)]
            prefetch_worker_starts: 0,
        })
    }

    /// Keep exactly one future part open in the worker's bounded queue.
    fn prefetch_next(&mut self) {
        if self.prefetched || self.next_part >= self.parts.len() {
            return;
        }
        if self.worker.is_none() {
            self.worker = NumericPartPrefetchWorker::new(self.columns.clone());
            if self.worker.is_none() {
                // Fallback to synchronous open when the OS cannot create a
                // prefetch thread. A resource limit must not abort the query.
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
            .and_then(|worker| worker.requests.as_ref())
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
                .ok_or_else(|| {
                    PartError::InvalidScan("missing numeric part prefetch worker".into())
                })?
                .responses
                .recv()
                .map_err(|_| {
                    PartError::InvalidScan("numeric snapshot part opener stopped".into())
                })??
        } else if self.next_part < self.parts.len() {
            let reader = NumericPartReader::open(&self.parts[self.next_part], &self.columns)?;
            self.next_part += 1;
            reader
        } else {
            return Ok(false);
        };
        self.current = Some(reader);
        self.prefetch_next();
        Ok(true)
    }

    /// Skip as much of an unfiltered OFFSET as possible using part/mark
    /// boundaries. No predicate or DISTINCT rows may be discarded this way.
    /// The result is the number of complete rows skipped, so the executor can
    /// still discard a remainder in the first decoded batch.
    pub fn skip_prefix_rows(&mut self, max_rows: usize) -> Result<usize, PartError> {
        if self.next_part != 0 || self.current.is_some() {
            return Err(PartError::InvalidScan(
                "OFFSET mark skipping requires a fresh reader".into(),
            ));
        }
        let mut skipped = 0usize;
        while skipped < max_rows && self.next_part < self.parts.len() {
            // Immutable part row counts are validated by the snapshot planner.
            // Skipping a complete part never needs to open its wide streams or
            // decode thousands of mark entries.
            let part = &self.parts[self.next_part];
            let rows = read_part_row_count(part)?;
            if let Ok(rows) = usize::try_from(rows) {
                if rows <= max_rows - skipped {
                    skipped += rows;
                    self.next_part += 1;
                    continue;
                }
            }
            let mut reader = NumericPartReader::open(part, &self.columns)?;
            self.next_part += 1;
            let amount = reader.skip_complete_marks(max_rows - skipped);
            skipped += amount;
            if u64::try_from(amount).ok() != Some(reader.rows()) {
                self.current = Some(reader);
                break;
            }
        }
        Ok(skipped)
    }

    pub fn next_batch(&mut self) -> Result<Option<Vec<NumericColumn>>, PartError> {
        // OFFSET may have already installed a partially skipped part.
        if self.current.is_some() {
            self.prefetch_next();
        }
        loop {
            if let Some(current) = self.current.as_mut() {
                if let Some(batch) = current.next_batch()? {
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

impl Drop for NumericQueryReader {
    fn drop(&mut self) {
        // Wait for any active open, including when LIMIT exits early.
        self.worker.take();
    }
}
