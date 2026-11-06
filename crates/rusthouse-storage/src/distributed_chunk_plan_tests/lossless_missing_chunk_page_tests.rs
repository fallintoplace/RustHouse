use super::*;
use crate::distributed_resume_map::ResumeMap;

#[test]
fn every_missing_chunk_survives_sequential_byte_limited_pages() {
    let map = ResumeMap::new(5).unwrap();
    let mut start = 0usize;
    let mut planned = Vec::new();
    for _ in 0..6 {
        let page =
            plan_missing_chunk_page_from_resume(29, 7, 65_536, &map, start, 8, 3).unwrap();
        planned.extend(page.chunks.iter().map(|chunk| chunk.index));
        match page.next_start_index {
            Some(next) => {
                assert!(next > start);
                start = next;
            }
            None => break,
        }
    }
    assert_eq!(planned, [0, 1, 2, 3, 4]);
    let mut completed = map;
    completed.mark_complete_range(0, 3).unwrap();
    let page = plan_missing_chunk_page_from_resume(29, 7, 65_536, &completed, 0, 8, 3).unwrap();
    assert_eq!(
        page.chunks
            .iter()
            .map(|chunk| chunk.index)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert!(page.next_start_index.is_none());
}

#[test]
fn unaffordable_first_chunk_cannot_advance_past_it() {
    let map = ResumeMap::new(3).unwrap();
    assert!(plan_missing_chunk_page_from_resume(15, 7, 65_536, &map, 0, 6, 9,).is_err());
    let final_page = plan_missing_chunk_page_from_resume(15, 7, 65_536, &map, 2, 6, 9).unwrap();
    assert_eq!(final_page.chunks[0].index, 2);
    assert!(final_page.next_start_index.is_none());
}

#[test]
fn rejects_mismatched_checkpoints_and_invalid_cursors() {
    let empty = ResumeMap::new(0).unwrap();
    let page = plan_missing_chunk_page_from_resume(0, 7, 65_536, &empty, 0, 0, 1).unwrap();
    assert!(page.chunks.is_empty());
    assert!(page.next_start_index.is_none());
    assert!(plan_missing_chunk_page_from_resume(8, 7, 65_536, &empty, 0, 7, 1,).is_err());
    assert!(plan_missing_chunk_page_from_resume(0, 7, 65_536, &empty, 1, 7, 1,).is_err());
    assert!(plan_missing_chunk_page_from_resume(0, 7, 65_536, &empty, 0, 7, 0,).is_err());
}
