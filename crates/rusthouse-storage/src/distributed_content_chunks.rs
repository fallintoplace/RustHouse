//! Stream-stable content-defined chunk boundaries for immutable objects.
//! Rolling hashes locate candidate boundaries, not cryptographic integrity.

const WINDOW: usize = 48;
const MAX_OBJECT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CHUNKS: usize = 65_536;
const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;

const fn byte_table() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut index = 0;
    while index < table.len() {
        let mut x = (index as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        table[index] = x ^ (x >> 31);
        index += 1;
    }
    table
}

const BYTE_TABLE: [u64; 256] = byte_table();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentChunk {
    pub offset: u64,
    pub length: u64,
    /// Advisory rolling fingerprint at the cut position, not a checksum.
    pub fingerprint: u64,
}

#[derive(Clone, Copy)]
pub struct ContentChunker {
    min_bytes: u64,
    average_mask: u64,
    max_bytes: u64,
    recent: [u8; WINDOW],
    next_slot: usize,
    filled: usize,
    rolling: u64,
    position: u64,
    start: u64,
    emitted: usize,
}

impl ContentChunker {
    /// Average size must be a power of two for unbiased mask boundaries.
    pub fn new(
        min_bytes: usize,
        average_bytes: usize,
        max_bytes: usize,
    ) -> Result<Self, &'static str> {
        if min_bytes < WINDOW
            || average_bytes < min_bytes
            || !average_bytes.is_power_of_two()
            || max_bytes < average_bytes
            || max_bytes > MAX_CHUNK_BYTES
        {
            return Err("invalid content-defined chunk sizes");
        }
        Ok(Self {
            min_bytes: min_bytes as u64,
            average_mask: average_bytes as u64 - 1,
            max_bytes: max_bytes as u64,
            recent: [0; WINDOW],
            next_slot: 0,
            filled: 0,
            rolling: 0,
            position: 0,
            start: 0,
            emitted: 0,
        })
    }

    fn add_byte(&mut self, byte: u8) -> Result<Option<ContentChunk>, &'static str> {
        let mut rolling = self.rolling.rotate_left(1) ^ BYTE_TABLE[byte as usize];
        if self.filled == WINDOW {
            rolling ^= BYTE_TABLE[self.recent[self.next_slot] as usize].rotate_left(WINDOW as u32);
        } else {
            self.filled += 1;
        }
        self.recent[self.next_slot] = byte;
        self.next_slot = (self.next_slot + 1) % WINDOW;
        self.rolling = rolling;
        self.position += 1;
        let length = self.position - self.start;
        if length < self.min_bytes {
            return Ok(None);
        }
        if rolling & self.average_mask != 0 && length < self.max_bytes {
            return Ok(None);
        }
        if self.emitted == MAX_CHUNKS {
            return Err("content chunk count exceeds limit");
        }
        self.emitted += 1;
        let chunk = ContentChunk {
            offset: self.start,
            length,
            fingerprint: self.rolling,
        };
        self.start = self.position;
        Ok(Some(chunk))
    }

    /// Process one fragment atomically. The same bytes produce the same
    /// boundaries regardless of the input fragment sizes.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<ContentChunk>, &'static str> {
        let total = self
            .position
            .checked_add(bytes.len() as u64)
            .ok_or("content chunk position overflow")?;
        if total > MAX_OBJECT_BYTES {
            return Err("content chunk source exceeds limit");
        }
        let mut next = *self;
        let mut chunks = Vec::new();
        for &byte in bytes {
            if let Some(chunk) = next.add_byte(byte)? {
                chunks.push(chunk);
            }
        }
        *self = next;
        Ok(chunks)
    }

    /// Return the final partial chunk, if any, without altering the state.
    pub fn finish(&self) -> Result<Option<ContentChunk>, &'static str> {
        if self.start == self.position {
            return Ok(None);
        }
        if self.emitted == MAX_CHUNKS {
            return Err("content chunk count exceeds limit");
        }
        Ok(Some(ContentChunk {
            offset: self.start,
            length: self.position - self.start,
            fingerprint: self.rolling,
        }))
    }
}

/// Convenience API for fully resident immutable object buffers.
pub fn plan_content_chunks(
    bytes: &[u8],
    min_bytes: usize,
    average_bytes: usize,
    max_bytes: usize,
) -> Result<Vec<ContentChunk>, &'static str> {
    let mut chunker = ContentChunker::new(min_bytes, average_bytes, max_bytes)?;
    let mut chunks = chunker.push(bytes)?;
    if let Some(last) = chunker.finish()? {
        chunks.push(last);
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let mut x = 0x1234_5678u32;
        (0..100_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 16) as u8
            })
            .collect()
    }

    #[test]
    fn fragment_sizes_do_not_change_content_boundaries() {
        let bytes = fixture();
        let whole = plan_content_chunks(&bytes, 128, 512, 4096).unwrap();
        let mut stream = ContentChunker::new(128, 512, 4096).unwrap();
        let mut streamed = Vec::new();
        for piece in bytes.chunks(77) {
            streamed.extend(stream.push(piece).unwrap());
        }
        streamed.extend(stream.finish().unwrap());
        assert_eq!(whole, streamed);
        assert!(whole.len() > 10);
        assert_eq!(whole[0].offset, 0);
        let mut end = 0;
        for chunk in &whole {
            assert_eq!(chunk.offset, end);
            assert!(chunk.length > 0 && chunk.length <= 4096);
            end += chunk.length;
        }
        assert_eq!(end, bytes.len() as u64);
    }

    #[test]
    fn empty_and_trailing_short_chunks_are_supported() {
        assert!(plan_content_chunks(b"", 64, 128, 512).unwrap().is_empty());
        assert_eq!(
            plan_content_chunks(b"short", 64, 128, 512).unwrap()[0].length,
            5
        );
        let mut chunker = ContentChunker::new(64, 128, 512).unwrap();
        assert_eq!(chunker.push(b"short").unwrap(), Vec::new());
        assert_eq!(chunker.finish().unwrap().unwrap().length, 5);
    }

    #[test]
    fn invalid_settings_and_oversized_push_preserve_state() {
        assert!(ContentChunker::new(1, 128, 512).is_err());
        assert!(ContentChunker::new(64, 100, 512).is_err());
        assert!(ContentChunker::new(256, 128, 512).is_err());
        assert!(ContentChunker::new(64, 128, 16).is_err());
        assert!(ContentChunker::new(64, 128, MAX_CHUNK_BYTES + 1).is_err());
        let chunker = ContentChunker::new(64, 128, 512).unwrap();
        let before = chunker.finish().unwrap();
        let invalid = vec![0; MAX_CHUNK_BYTES + 1];
        let mut short_bound = chunker;
        short_bound.position = MAX_OBJECT_BYTES - 1;
        assert!(short_bound.push(&invalid).is_err());
        assert_eq!(chunker.finish().unwrap(), before);
    }

    #[test]
    fn forced_boundaries_remain_bounded_on_uniform_input() {
        let bytes = vec![0u8; 10_001];
        let chunks = plan_content_chunks(&bytes, 64, 128, 512).unwrap();
        assert!(chunks.iter().all(|chunk| chunk.length <= 512));
        assert_eq!(chunks.iter().map(|chunk| chunk.length).sum::<u64>(), 10_001);
    }
}


/// Exact in-memory reuse of source bytes or transfer of target bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkDeltaOperation {
    Reuse {
        source_offset: u64,
        target_offset: u64,
        length: u64,
    },
    Copy {
        target_offset: u64,
        length: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkDeltaPlan {
    pub operations: Vec<ChunkDeltaOperation>,
    pub bytes_reused: u64,
    pub bytes_to_copy: u64,
}

fn chunk_lookup_hash(bytes: &[u8]) -> u64 {
    // A fast lookup hint only. Reuse ALWAYS requires exact byte equality.
    let mut state = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        state = (state ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
    }
    state
}

/// Plan exact reusable chunks when an immutable object changes.
///
/// Uses content-defined boundaries and lookup hashes to find candidates,
/// then compares full bytes before reusing source offsets. Hash collisions
/// can reduce performance but cannot lead to incorrectly reused data.
/// Adjacent changed target chunks are coalesced into one transfer range.
pub fn plan_verified_chunk_delta(
    source: &[u8],
    target: &[u8],
    min_bytes: usize,
    average_bytes: usize,
    max_bytes: usize,
) -> Result<ChunkDeltaPlan, &'static str> {
    use std::collections::HashMap;

    let old_chunks = plan_content_chunks(source, min_bytes, average_bytes, max_bytes)?;
    let new_chunks = plan_content_chunks(target, min_bytes, average_bytes, max_bytes)?;
    let mut candidates: HashMap<(u64, u64), Vec<u64>> =
        HashMap::with_capacity(old_chunks.len());
    for chunk in old_chunks {
        let begin = chunk.offset as usize;
        let end = (chunk.offset + chunk.length) as usize;
        let key = (chunk.length, chunk_lookup_hash(&source[begin..end]));
        candidates.entry(key).or_default().push(chunk.offset);
    }

    let mut operations = Vec::with_capacity(new_chunks.len());
    let mut bytes_reused = 0u64;
    let mut bytes_to_copy = 0u64;
    for chunk in new_chunks {
        let begin = chunk.offset as usize;
        let end = (chunk.offset + chunk.length) as usize;
        let bytes = &target[begin..end];
        let key = (chunk.length, chunk_lookup_hash(bytes));
        let old_offset = candidates.get(&key).and_then(|offsets| {
            offsets.iter().copied().find(|&offset| {
                let start = offset as usize;
                let finish = start + bytes.len();
                &source[start..finish] == bytes
            })
        });
        if let Some(source_offset) = old_offset {
            operations.push(ChunkDeltaOperation::Reuse {
                source_offset,
                target_offset: chunk.offset,
                length: chunk.length,
            });
            bytes_reused += chunk.length;
        } else {
            bytes_to_copy += chunk.length;
            if let Some(ChunkDeltaOperation::Copy {
                target_offset,
                length,
            }) = operations.last_mut()
            {
                if *target_offset + *length == chunk.offset {
                    *length += chunk.length;
                    continue;
                }
            }
            operations.push(ChunkDeltaOperation::Copy {
                target_offset: chunk.offset,
                length: chunk.length,
            });
        }
    }
    debug_assert_eq!(bytes_reused + bytes_to_copy, target.len() as u64);
    Ok(ChunkDeltaPlan {
        operations,
        bytes_reused,
        bytes_to_copy,
    })
}

#[cfg(test)]
mod chunk_delta_tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let mut value = 123456789u32;
        (0..50_000)
            .map(|_| {
                value = value.wrapping_mul(1664525).wrapping_add(1013904223);
                (value >> 16) as u8
            })
            .collect()
    }

    fn reconstruct(source: &[u8], target: &[u8], plan: &ChunkDeltaPlan) -> Vec<u8> {
        let mut output = vec![0; target.len()];
        for operation in &plan.operations {
            match *operation {
                ChunkDeltaOperation::Reuse {
                    source_offset,
                    target_offset,
                    length,
                } => {
                    let from = source_offset as usize;
                    let to = target_offset as usize;
                    output[to..to + length as usize]
                        .copy_from_slice(&source[from..from + length as usize]);
                }
                ChunkDeltaOperation::Copy {
                    target_offset,
                    length,
                } => {
                    let to = target_offset as usize;
                    output[to..to + length as usize]
                        .copy_from_slice(&target[to..to + length as usize]);
                }
            }
        }
        output
    }

    #[test]
    fn insertion_and_local_edits_reuse_unchanged_regions() {
        let source = fixture();
        let mut target = b"inserted-prefix".to_vec();
        target.extend_from_slice(&source);
        target[20_000] ^= 1;
        let plan = plan_verified_chunk_delta(&source, &target, 48, 128, 512).unwrap();
        assert_eq!(reconstruct(&source, &target, &plan), target);
        assert!(plan.bytes_reused > 20_000);
        assert!(plan.bytes_to_copy < target.len() as u64);
        assert_eq!(plan.bytes_reused + plan.bytes_to_copy, target.len() as u64);
        assert_eq!(
            plan,
            plan_verified_chunk_delta(&source, &target, 48, 128, 512).unwrap()
        );
    }

    #[test]
    fn empty_and_invalid_configs_are_safe() {
        let empty = plan_verified_chunk_delta(&[], &[], 48, 128, 512).unwrap();
        assert!(empty.operations.is_empty());
        let new = plan_verified_chunk_delta(&[], b"hello", 48, 128, 512).unwrap();
        assert_eq!(new.bytes_to_copy, 5);
        assert_eq!(new.bytes_reused, 0);
        assert!(plan_verified_chunk_delta(b"abc", b"xyz", 0, 128, 512).is_err());
        assert!(plan_verified_chunk_delta(b"abc", b"xyz", 48, 127, 512).is_err());
    }
}
