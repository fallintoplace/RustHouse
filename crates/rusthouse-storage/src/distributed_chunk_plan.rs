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
    let count = total_bytes / chunk_bytes + u64::from(total_bytes % chunk_bytes != 0);
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
    fn handles_the_largest_u64_range() {
        assert_eq!(
            plan_chunks(u64::MAX, u64::MAX, 1).unwrap(),
            vec![Chunk {
                offset: 0,
                length: u64::MAX
            }]
        );
    }
}