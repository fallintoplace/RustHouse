use super::*;

#[test]
fn combines_worker_progress_without_losing_bit_positions() {
    let mut first = ResumeMap::new(10).unwrap();
    first.mark_complete(0).unwrap();
    first.mark_complete(8).unwrap();
    let mut second = ResumeMap::new(10).unwrap();
    second.mark_complete(1).unwrap();
    second.mark_complete(9).unwrap();
    first.merge_verified(&second).unwrap();
    assert_eq!(first.remaining(), vec![2, 3, 4, 5, 6, 7]);
    assert_eq!(ResumeMap::decode(&first.encode()).unwrap(), first);
    first.merge_verified(&second).unwrap();
    assert_eq!(first.remaining().len(), 6);
}

#[test]
fn incompatible_maps_leave_progress_unchanged() {
    let mut first = ResumeMap::new(8).unwrap();
    first.mark_complete(7).unwrap();
    let original = first.clone();
    assert!(first.merge_verified(&ResumeMap::new(9).unwrap()).is_err());
    assert_eq!(first, original);
    assert!(
        ResumeMap::new(0)
            .unwrap()
            .merge_verified(&ResumeMap::new(0).unwrap())
            .is_ok()
    );
}
