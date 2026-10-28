//! Work-stealing, mergeable HLL on immutable wide-part snapshots.
//! Scans separate parts concurrently, then reduces fixed-size HLL registers
//! with maxima. No cardinality-sized HashSet or per-row result materialization.
use super::hyperloglog::HyperLogLog;
use super::typed;
use super::ExecutionError;
use crate::{Projection, Select};
use rusthouse_storage::{TypedQueryReader, inspect_query_source, query_parts};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const MIN_PARTS: usize = 4;
const MAX_WORKERS: usize = 8;

pub(super) fn try_execute(
    query: &Select,
    source: &Path,
) -> Result<Option<u64>, ExecutionError> {
    let Projection::ApproxDistinct { column } = &query.projection else {
        return Ok(None);
    };
    if query.limit == Some(0) || query.offset.unwrap_or(0) > 0 {
        return Ok(None);
    }
    // Validate the complete immutable input and all schemas before spawning
    // workers. A partially readable part must fail the whole query.
    let metadata = inspect_query_source(source)?;
    let parts = query_parts(source)?;
    let workers = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(MAX_WORKERS)
        .min(parts.len());
    if workers < 2 || parts.len() < MIN_PARTS {
        return Ok(None);
    }
    if !metadata.columns.iter().any(|candidate| candidate.name == *column) {
        return Err(ExecutionError::Invalid(format!(
            "unknown uniq column: {column}"
        )));
    }
    let mut names = vec![column.clone()];
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        if !names.iter().any(|name| name == predicate.column()) {
            names.push(predicate.column().to_owned());
        }
    }
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<String, usize>>();
    let types = metadata
        .columns
        .iter()
        .map(|candidate| (candidate.name.clone(), candidate.data_type.clone()))
        .collect::<HashMap<String, String>>();
    let cursor = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let value_index = indices[column];

    let results = std::thread::scope(|scope| {
        let handles = (0..workers)
            .map(|_| {
                let names = &names;
                let parts = &parts;
                let indices = &indices;
                let types = &types;
                let cursor = &cursor;
                let failed = &failed;
                scope.spawn(move || -> Result<HyperLogLog, ExecutionError> {
                    let mut conditions = vec![typed::compile(&query.predicates, indices, types)?];
                    for branch in &query.or_predicates {
                        conditions.push(typed::compile(branch, indices, types)?);
                    }
                    let mut sketch = HyperLogLog::default();
                    while !failed.load(Ordering::Relaxed) {
                        let index = cursor.fetch_add(1, Ordering::Relaxed);
                        let Some(part) = parts.get(index) else {
                            break;
                        };
                        // Check each immutable part's physical marks and batch
                        // layout through the existing validated typed reader.
                        let scan = (|| -> Result<(), ExecutionError> {
                            let mut reader = TypedQueryReader::open(part, names)?;
                            while let Some(batch) = reader.next_batch()? {
                                for row in 0..batch[0].len() {
                                    if !no_filters && !typed::passes(&batch, row, &conditions) {
                                        continue;
                                    }
                                    sketch.observe(&batch[value_index], row)?;
                                }
                            }
                            Ok(())
                        })();
                        if let Err(error) = scan {
                            failed.store(true, Ordering::Relaxed);
                            return Err(error);
                        }
                    }
                    Ok(sketch)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle.join().map_err(|_| {
                    ExecutionError::Invalid("parallel uniq worker panicked".into())
                })
            })
            .collect::<Result<Vec<_>, ExecutionError>>()
    })?;

    let mut merged = HyperLogLog::default();
    for result in results {
        merged.merge(&result?);
    }
    Ok(Some(merged.estimate()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_budget_is_not_proportional_to_part_count() {
        assert!(MAX_WORKERS <= 8);
        assert_eq!(MIN_PARTS, 4);
    }
}
