use super::*;

#[test]
fn smaller_tail_can_progress_when_first_chunk_does_not_fit() {
    assert_eq!(
        plan_missing_chunks(10, 4, 3, &[0, 2], 2, 2).unwrap(),
        vec![IndexedChunk {
            index: 2,
            chunk: Chunk {
                offset: 8,
                length: 2
            }
        }]
    );
    assert!(
        plan_missing_chunks(10, 4, 3, &[0, 2], 1, 2)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        plan_missing_chunks(10, 4, 3, &[0, 1, 2], 6, 3)
            .unwrap()
            .len(),
        2
    );
}
