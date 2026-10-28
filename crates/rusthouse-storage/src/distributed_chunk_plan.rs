//! Deterministic byte ranges for resumable immutable-object copies.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    pub offset: u64,
    pub length: u64,
}

/// Plan nonoverlapping ranges. Empty files have no data chunks.
pub fn plan_chunks(
    total_bytes: u64,
    chunk_bytes: u64,
    max_chunks: usize,
) -> Result<Vec<Chunk>, &'static str> {
    if chunk_bytes == 0 || max_chunks == 0 || max_chunks > 65_536 {
        return Err("invalid transfer chunk limits");
    }
    let count = total_bytes.div_ceil(chunk_bytes);
    if count > max_chunks as u64 {
        return Err("transfer exceeds chunk count limit");
    }
    let mut chunks = Vec::with_capacity(count as usize);
    let mut offset = 0u64;
    while offset < total_bytes {
        let length = (total_bytes - offset).min(chunk_bytes);
        chunks.push(Chunk { offset, length });
        offset += length;
    }
    Ok(chunks)
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
    let chunks = plan_chunks(total_bytes, chunk_bytes, max_chunks)?;
    if remaining_indices.iter().any(|&index| index >= chunks.len())
        || remaining_indices.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err("invalid or duplicate missing chunk index");
    }
    let mut selected = Vec::new();
    let mut remaining_bytes = byte_budget;
    for &index in remaining_indices {
        if selected.len() >= max_batch_chunks || chunks[index].length > remaining_bytes {
            break;
        }
        let chunk = chunks[index];
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
                Chunk { offset: 0, length: 4 },
                Chunk { offset: 4, length: 4 },
                Chunk { offset: 8, length: 2 },
            ]
        );
        assert!(plan_chunks(0, 4, 1).unwrap().is_empty());
        assert!(plan_chunks(10, 4, 2).is_err());
        assert!(plan_chunks(10, 0, 10).is_err());
    }

    #[test]
    fn schedules_only_missing_chunks_with_stable_indices_and_byte_budget() {
        let pending = plan_missing_chunks(10, 4, 3, &[0, 2], 6, 3).unwrap();
        assert_eq!(pending, vec![
            IndexedChunk { index: 0, chunk: Chunk { offset: 0, length: 4 } },
            IndexedChunk { index: 2, chunk: Chunk { offset: 8, length: 2 } },
        ]);
        assert_eq!(
            plan_missing_chunks(10, 4, 3, &[0, 1, 2], 7, 3).unwrap(),
            vec![IndexedChunk { index: 0, chunk: Chunk { offset: 0, length: 4 } }],
        );
        assert_eq!(
            plan_missing_chunks(10, 4, 3, &[2], 2, 1).unwrap(),
            vec![IndexedChunk { index: 2, chunk: Chunk { offset: 8, length: 2 } }],
        );
        assert!(plan_missing_chunks(0, 4, 3, &[], 0, 3).unwrap().is_empty());
        assert!(plan_missing_chunks(10, 4, 3, &[2], 0, 3).unwrap().is_empty());
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
            vec![Chunk { offset: 0, length: u64::MAX }]
        );
    }
}