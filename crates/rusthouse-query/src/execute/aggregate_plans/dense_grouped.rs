use super::*;

struct DenseGroupChunk {
    counts: Vec<u64>,
    order: Vec<u16>,
}

/// Parallelize low-cardinality GROUP BY over many immutable parts. Each worker
/// keeps fixed-size counters rather than one hash entry per input row.
/// Merge chunks in part order to retain stable first-seen output order.
pub(super) fn parallel_dense_grouped_count(
    part: &Path,
    column: &str,
    kind: &str,
) -> Result<Option<GroupedCounts>, ExecutionError> {
    let parts = rusthouse_storage::query_parts(part)?;
    let workers = super::super::query_workers(4);
    if workers < 2 || parts.len() < 16 {
        return Ok(None);
    }
    let slots = if matches!(kind, "Int8" | "UInt8") {
        256
    } else {
        65_536
    };
    let names = [column.to_owned()];
    let chunk_size = parts.len().div_ceil(workers);
    let chunks = std::thread::scope(|scope| -> Result<_, ExecutionError> {
        let names = &names;
        let handles = parts
            .chunks(chunk_size)
            .map(|paths| {
                std::thread::Builder::new()
                    .spawn_scoped(scope, move || -> Result<DenseGroupChunk, ExecutionError> {
                        let mut counts = vec![0u64; slots];
                        let mut order = Vec::new();
                        for path in paths {
                            let mut reader =
                                rusthouse_storage::NumericPartReader::open(path, names)?;
                            while let Some(batch) = reader.next_batch()? {
                                match &batch[0] {
                                    NumericColumn::UInt8(values) => {
                                        for &value in values {
                                            let slot = usize::from(value);
                                            if counts[slot] == 0 {
                                                order.push(value.into());
                                            }
                                            counts[slot] =
                                                counts[slot].checked_add(1).ok_or_else(|| {
                                                    ExecutionError::Invalid(
                                                        "group count overflow".into(),
                                                    )
                                                })?;
                                        }
                                    }
                                    NumericColumn::Int8(values) => {
                                        for &value in values {
                                            let slot = usize::from(value as u8);
                                            if counts[slot] == 0 {
                                                order.push(slot as u16);
                                            }
                                            counts[slot] =
                                                counts[slot].checked_add(1).ok_or_else(|| {
                                                    ExecutionError::Invalid(
                                                        "group count overflow".into(),
                                                    )
                                                })?;
                                        }
                                    }
                                    NumericColumn::UInt16(values) => {
                                        for &value in values {
                                            let slot = usize::from(value);
                                            if counts[slot] == 0 {
                                                order.push(value);
                                            }
                                            counts[slot] =
                                                counts[slot].checked_add(1).ok_or_else(|| {
                                                    ExecutionError::Invalid(
                                                        "group count overflow".into(),
                                                    )
                                                })?;
                                        }
                                    }
                                    NumericColumn::Int16(values) => {
                                        for &value in values {
                                            let slot = usize::from(value as u16);
                                            if counts[slot] == 0 {
                                                order.push(slot as u16);
                                            }
                                            counts[slot] =
                                                counts[slot].checked_add(1).ok_or_else(|| {
                                                    ExecutionError::Invalid(
                                                        "group count overflow".into(),
                                                    )
                                                })?;
                                        }
                                    }
                                    _ => {
                                        return Err(ExecutionError::Invalid(
                                            "unexpected grouped key type".into(),
                                        ));
                                    }
                                }
                            }
                        }
                        Ok(DenseGroupChunk { counts, order })
                    })
                    .map_err(|error| {
                        ExecutionError::Invalid(format!(
                            "cannot start dense integer GROUP BY worker: {error}"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| {
                    Err(ExecutionError::Invalid(
                        "grouped scan worker panicked".into(),
                    ))
                })
            })
            .collect::<Vec<_>>())
    })?;
    let mut counts = vec![0u64; slots];
    let mut order = Vec::new();
    for chunk in chunks {
        let chunk = chunk?;
        for key in chunk.order {
            let slot = usize::from(key);
            if counts[slot] == 0 {
                order.push(key);
            }
            counts[slot] = counts[slot]
                .checked_add(chunk.counts[slot])
                .ok_or_else(|| ExecutionError::Invalid("group count overflow".into()))?;
        }
    }
    let mut groups = GroupedCounts::default();
    for key in order {
        let value = match kind {
            "Int8" => Number::Signed(i128::from(key as u8 as i8)),
            "Int16" => Number::Signed(i128::from(key as i16)),
            _ => Number::Unsigned(u128::from(key)),
        };
        groups.rows.push((value, counts[usize::from(key)]));
    }
    Ok(Some(groups))
}
