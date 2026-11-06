use super::*;

#[test]
fn consecutive_runs_and_completed_gaps_keep_original_indices() {
    let chunks = [
        IndexedChunk {
            index: 0,
            chunk: Chunk {
                offset: 0,
                length: 4,
            },
        },
        IndexedChunk {
            index: 1,
            chunk: Chunk {
                offset: 4,
                length: 4,
            },
        },
        IndexedChunk {
            index: 4,
            chunk: Chunk {
                offset: 16,
                length: 4,
            },
        },
        IndexedChunk {
            index: 5,
            chunk: Chunk {
                offset: 20,
                length: 4,
            },
        },
    ];
    assert_eq!(
        coalesce_indexed_chunks(&chunks, 8).unwrap(),
        vec![
            ChunkRun {
                first_index: 0,
                chunk_count: 2,
                offset: 0,
                length: 8
            },
            ChunkRun {
                first_index: 4,
                chunk_count: 2,
                offset: 16,
                length: 8
            },
        ]
    );
}

#[test]
fn late_invalid_chunk_rejects_whole_coalescing_plan() {
    let chunks = [
        IndexedChunk {
            index: 0,
            chunk: Chunk {
                offset: 0,
                length: 4,
            },
        },
        IndexedChunk {
            index: 1,
            chunk: Chunk {
                offset: 4,
                length: 4,
            },
        },
        IndexedChunk {
            index: 2,
            chunk: Chunk {
                offset: u64::MAX,
                length: 2,
            },
        },
    ];
    assert_eq!(
        coalesce_indexed_chunks(&chunks, 8),
        Err("transfer chunk range overflow")
    );
}
