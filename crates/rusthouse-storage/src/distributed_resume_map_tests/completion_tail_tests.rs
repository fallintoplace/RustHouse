use super::*;

#[test]
fn completion_handles_all_partial_bitmap_widths() {
    for length in 1..8 {
        let mut map = ResumeMap::new(length).unwrap();
        assert!(!map.complete());
        map.mark_complete_range(0, length - 1).unwrap();
        assert!(!map.complete());
        map.mark_complete(length - 1).unwrap();
        assert!(map.complete());
    }
}
