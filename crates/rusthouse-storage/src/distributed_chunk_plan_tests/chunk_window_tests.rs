use super::*;

#[test]
fn pages_partial_tail_with_original_indexes() {
    assert_eq!(
        plan_chunk_window(10, 4, 3, 1, 2).unwrap(),
        vec![
            IndexedChunk {
                index: 1,
                chunk: Chunk {
                    offset: 4,
                    length: 4
                },
            },
            IndexedChunk {
                index: 2,
                chunk: Chunk {
                    offset: 8,
                    length: 2
                },
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
