use super::*;

#[test]
fn dropping_worker_slot_releases_capacity() {
    let active = Arc::new(AtomicUsize::new(1));
    {
        let _permit = ActiveRequest(Arc::clone(&active));
        assert_eq!(active.load(Ordering::Acquire), 1);
    }
    assert_eq!(active.load(Ordering::Acquire), 0);
}
