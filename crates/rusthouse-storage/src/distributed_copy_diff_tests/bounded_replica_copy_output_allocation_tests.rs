use super::*;

#[test]
fn tiny_byte_pages_do_not_preallocate_the_full_missing_inventory() {
    let source = (0..8192)
        .map(|i| ObjectMetadata {
            key: format!("part/file-{i:05}"),
            bytes: 1_000_000,
            crc32c: i,
        })
        .collect::<Vec<_>>();
    let planner = ReplicaCopyPlanner::new(&source, &[]).unwrap();
    let batch = planner.batch_after(None, 1_000_000, 65_536).unwrap();
    assert_eq!(batch.len(), 1);
    assert!(batch.capacity() <= 64);
    let page = planner.page_after(None, 1_000_000, 65_536).unwrap();
    assert_eq!(page.copies.len(), 1);
    assert!(page.copies.capacity() <= 64);
    assert_eq!(page.next_after.as_deref(), Some("part/file-00000"));
}

#[test]
fn large_affordable_batches_still_contain_every_requested_object() {
    let source = (0..1024)
        .map(|i| ObjectMetadata {
            key: format!("shard/item-{i:05}"),
            bytes: 1,
            crc32c: i,
        })
        .collect::<Vec<_>>();
    let planner = ReplicaCopyPlanner::new(&source, &[]).unwrap();
    let page = planner.page_after(None, 1024, 1024).unwrap();
    assert_eq!(page.copies.len(), 1024);
    assert_eq!(page.next_after, None);
    let batch = planner.batch_after(None, 1024, 1024).unwrap();
    assert_eq!(batch, page.copies);
}
