use super::*;

#[test]
fn iterator_matches_materialized_progress_for_partial_and_full_bytes() {
    for chunks in [0, 1, 7, 8, 9, 63, 64, 65, 513, 65_536] {
        let mut map = ResumeMap::new(chunks).unwrap();
        assert_eq!(
            map.remaining_indices().collect::<Vec<_>>(),
            (0..chunks).collect::<Vec<_>>()
        );
        for i in (0..chunks).step_by(3) {
            map.mark_complete(i).unwrap();
        }
        let remaining = map.remaining_indices().collect::<Vec<_>>();
        assert_eq!(remaining, map.remaining());
        assert_eq!(remaining.len(), map.remaining_count());
        map.mark_complete_range(0, chunks).unwrap();
        assert_eq!(map.remaining_indices().count(), 0);
    }
}
