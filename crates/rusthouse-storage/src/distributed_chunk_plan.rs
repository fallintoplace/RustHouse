//! Deterministic byte ranges for resumable immutable-object copies.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    pub offset: u64,
    pub length: u64,
}

fn checked_chunk_count(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
) -> Result<usize, &'static str> {
    if chunk_bytes == 0 || max_chunks == 0 || max_chunks > 65_536 {
        return Err("invalid transfer chunk limits");
    }
    let count = total_bytes.div_ceil(chunk_bytes);
    if count > max_chunks as u64 {
        return Err("transfer exceeds chunk count limit");
    }
    Ok(count as usize)
}

fn chunk_at_index(total_bytes: u64, chunk_bytes: u64, index: usize) -> Chunk {
    let offset = index as u64 * chunk_bytes;
    Chunk {
        offset,
        length: (total_bytes - offset).min(chunk_bytes),
    }
}

/// Plan nonoverlapping ranges. Empty files have no data chunks.
pub fn plan_chunks(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
) -> Result<Vec<Chunk>, &'static str> {
    let count = checked_chunk_count(total_bytes, chunk_bytes, max_chunks)?;
    Ok((0..count)
        .map(|index| chunk_at_index(total_bytes, chunk_bytes, index))
        .collect())
}

/// A pending byte range plus its stable index in the complete transfer
/// plan. The index can be persisted in a ResumeMap after verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexedChunk {
    pub index: usize,
    pub chunk: Chunk,
}

/// Select an ordered, bounded prefix from ResumeMap::remaining().
///
/// Validate strictly increasing indexes so a corrupt progress checkpoint
/// cannot duplicate writes or silently omit ranges. No network work occurs.
pub fn plan_missing_chunks(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
    remaining_indices: &[usize],
    byte_budget: u64,
    max_batch_chunks: usize,
) -> Result<Vec<IndexedChunk>, &'static str> {
    if max_batch_chunks > 65_536 {
        return Err("transfer batch chunk limit exceeds bound");
    }
    let chunk_count = checked_chunk_count(total_bytes, chunk_bytes, max_chunks)?;
    let mut previous_chunk_index = None;
    for &index in remaining_indices {
        if index >= chunk_count || previous_chunk_index.is_some_and(|last| index <= last) {
            return Err("invalid or duplicate missing chunk index");
        }
        previous_chunk_index = Some(index);
    }
    // Only full chunks plus possibly one short final chunk can fit this budget.
    let budget_chunks = byte_budget / chunk_bytes;
    let reserve = remaining_indices
        .len()
        .min(max_batch_chunks)
        .min(usize::try_from(budget_chunks.saturating_add(1)).unwrap_or(usize::MAX));
    let mut selected = Vec::with_capacity(reserve);
    let mut remaining_bytes = byte_budget;
    for &index in remaining_indices {
        if selected.len() >= max_batch_chunks {
            break;
        }
        let chunk = chunk_at_index(total_bytes, chunk_bytes, index);
        if chunk.length > remaining_bytes {
            continue;
        }
        remaining_bytes -= chunk.length;
        selected.push(IndexedChunk { index, chunk });
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partitions_without_gaps_or_overlaps() {
        assert_eq!(
            plan_chunks(10, 4, 3).unwrap(),
            vec![
                Chunk {
                    offset: 0,
                    length: 4
                },
                Chunk {
                    offset: 4,
                    length: 4
                },
                Chunk {
                    offset: 8,
                    length: 2
                },
            ]
        );
        assert!(plan_chunks(0, 4, 1).unwrap().is_empty());
        assert!(plan_chunks(10, 4, 2).is_err());
        assert!(plan_chunks(10, 0, 10).is_err());
    }

    #[test]
    fn schedules_only_missing_chunks_with_stable_indices_and_byte_budget() {
        let pending = plan_missing_chunks(10, 4, 3, &[0, 2], 6, 3).unwrap();
        assert_eq!(
            pending,
            vec![
                IndexedChunk {
                    index: 0,
                    chunk: Chunk {
                        offset: 0,
                        length: 4
                    }
                },
                IndexedChunk {
                    index: 2,
                    chunk: Chunk {
                        offset: 8,
                        length: 2
                    }
                },
            ]
        );
        assert_eq!(
            plan_missing_chunks(10, 4, 3, &[0, 1, 2], 7, 3).unwrap(),
            vec![
                IndexedChunk { index: 0, chunk: Chunk { offset: 0, length: 4 } },
                IndexedChunk { index: 2, chunk: Chunk { offset: 8, length: 2 } },
            ],
        );
        assert_eq!(
            plan_missing_chunks(10, 4, 3, &[2], 2, 1).unwrap(),
            vec![IndexedChunk {
                index: 2,
                chunk: Chunk {
                    offset: 8,
                    length: 2
                }
            }],
        );
        assert!(plan_missing_chunks(0, 4, 3, &[], 0, 3).unwrap().is_empty());
        assert!(
            plan_missing_chunks(10, 4, 3, &[2], 0, 3)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn tiny_resume_budget_does_not_reserve_full_batch() {
        let remaining = (0..65_536).collect::<Vec<_>>();
        let chunks = plan_missing_chunks(65_536, 1, 65_536, &remaining, 0, 65_536).unwrap();
        assert!(chunks.is_empty());
        assert!(chunks.capacity() <= 1);
        let last = plan_missing_chunks(9, 4, 3, &[2], 1, 3).unwrap();
        assert_eq!(last.len(), 1);
    }

    #[test]
    fn validates_sparse_indexes_before_selecting_bounded_transfer_prefix() {
        let selected = plan_missing_chunks(101, 1, 128, &[0, 2, 7, 63, 64, 100], 3, 3).unwrap();
        assert_eq!(
            selected.iter().map(|part| part.index).collect::<Vec<_>>(),
            vec![0, 2, 7]
        );
        assert!(plan_missing_chunks(101, 1, 128, &[0, 2, 7, 7], 4, 4).is_err());
        assert!(plan_missing_chunks(101, 1, 128, &[0, 2, 101], 4, 4).is_err());
    }

    #[test]
    fn rejects_corrupt_missing_chunk_indexes() {
        for indexes in [&[0, 0][..], &[2, 1][..], &[3][..]] {
            assert!(plan_missing_chunks(10, 4, 3, indexes, 10, 3).is_err());
        }
        assert!(plan_missing_chunks(10, 4, 3, &[0], 10, 65_537).is_err());
        assert!(plan_missing_chunks(10, 0, 3, &[0], 10, 3).is_err());
    }

    #[test]
    fn handles_the_largest_u64_range() {
        assert_eq!(
            plan_chunks(u64::MAX, u64::MAX, 1).unwrap(),
            vec![Chunk {
                offset: 0,
                length: u64::MAX
            }]
        );
    }
    #[test]
    fn sparse_resume_uses_stable_indexes_at_maximum_chunk_count() {
        let selected = plan_missing_chunks(65_536, 1, 65_536, &[0, 32_768, 65_535], 2, 3).unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(
            selected[0].chunk,
            Chunk {
                offset: 0,
                length: 1
            }
        );
        assert_eq!(
            selected[1],
            IndexedChunk {
                index: 32_768,
                chunk: Chunk {
                    offset: 32_768,
                    length: 1
                },
            }
        );
        assert!(plan_missing_chunks(65_537, 1, 65_536, &[], 0, 1).is_err());
    }
}

/// Page through a transfer plan without allocating the entire chunk list.
pub fn plan_chunk_window(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
    start_index: usize,
    requested: usize,
) -> Result<Vec<IndexedChunk>, &'static str> {
    let count = checked_chunk_count(total_bytes, chunk_bytes, max_chunks)?;
    if start_index > count || requested > count - start_index {
        return Err("transfer chunk window exceeds bounds");
    }
    let mut page = Vec::with_capacity(requested);
    for index in start_index..start_index + requested {
        page.push(IndexedChunk {
            index,
            chunk: chunk_at_index(total_bytes, chunk_bytes, index),
        });
    }
    Ok(page)
}

#[cfg(test)]
mod chunk_window_tests {
    use super::*;

    #[test]
    fn pages_partial_tail_with_original_indexes() {
        assert_eq!(
            plan_chunk_window(10, 4, 3, 1, 2).unwrap(),
            vec![
                IndexedChunk {
                    index: 1,
                    chunk: Chunk { offset: 4, length: 4 },
                },
                IndexedChunk {
                    index: 2,
                    chunk: Chunk { offset: 8, length: 2 },
                },
            ]
        );
        assert!(plan_chunk_window(10, 4, 3, 2, 2).is_err());
        assert!(plan_chunk_window(10, 4, 3, 4, 0).is_err());
        assert!(plan_chunk_window(0, 4, 3, 0, 0).unwrap().is_empty());
    }

    #[test]
    fn large_transfers_allocate_only_requested_chunks() {
        let page = plan_chunk_window(65_536, 1, 65_536, 65_534, 2).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[1].index, 65_535);
    }
}

#[cfg(test)]
mod skipped_chunk_budget_tests {
    use super::*;

    #[test]
    fn smaller_tail_can_progress_when_first_chunk_does_not_fit() {
        assert_eq!(
            plan_missing_chunks(10, 4, 3, &[0, 2], 2, 2).unwrap(),
            vec![IndexedChunk { index: 2, chunk: Chunk { offset: 8, length: 2 } }]
        );
        assert!(plan_missing_chunks(10, 4, 3, &[0, 2], 1, 2).unwrap().is_empty());
        assert_eq!(plan_missing_chunks(10, 4, 3, &[0, 1, 2], 6, 3).unwrap().len(), 2);
    }
}
