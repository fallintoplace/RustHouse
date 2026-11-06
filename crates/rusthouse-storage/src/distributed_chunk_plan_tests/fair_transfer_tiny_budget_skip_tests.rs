use super::*;
use crate::distributed_object_key::ObjectKey;
use crate::distributed_resume_map::ResumeMap;

fn key(file: &str) -> ObjectKey {
    ObjectKey::new("cluster", "shard", 1, "part", file).unwrap()
}

#[test]
fn tiny_budget_reaches_short_tails_without_scheduling_large_chunks() {
    let keys = [key("first"), key("second")];
    let maps = [ResumeMap::new(4097).unwrap(), ResumeMap::new(4097).unwrap()];
    let sources = [
        FairTransferSource {
            key: &keys[0],
            total_bytes: 4096 * 128 + 7,
            chunk_bytes: 128,
            checkpoint: &maps[0],
        },
        FairTransferSource {
            key: &keys[1],
            total_bytes: 4096 * 128 + 7,
            chunk_bytes: 128,
            checkpoint: &maps[1],
        },
    ];
    let selected = plan_fair_replica_transfers(&sources, 14, 16).unwrap();
    assert_eq!(selected.len(), 2);
    assert_eq!(
        (selected[0].source_index, selected[0].indexed.index),
        (0, 4096)
    );
    assert_eq!(
        (selected[1].source_index, selected[1].indexed.index),
        (1, 4096)
    );
    assert_eq!(
        selected
            .iter()
            .map(|item| item.indexed.chunk.length)
            .sum::<u64>(),
        14
    );
}

#[test]
fn completed_tail_does_not_fall_back_to_unaffordable_full_chunks() {
    let keys = [key("first"), key("second")];
    let mut first = ResumeMap::new(4097).unwrap();
    first.mark_complete(4096).unwrap();
    let second = ResumeMap::new(4097).unwrap();
    let sources = [
        FairTransferSource {
            key: &keys[0],
            total_bytes: 4096 * 128 + 7,
            chunk_bytes: 128,
            checkpoint: &first,
        },
        FairTransferSource {
            key: &keys[1],
            total_bytes: 4096 * 128 + 7,
            chunk_bytes: 128,
            checkpoint: &second,
        },
    ];
    let selected = plan_fair_replica_transfers(&sources, 14, 16).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(
        (selected[0].source_index, selected[0].indexed.index),
        (1, 4096)
    );
    assert_eq!(
        plan_fair_replica_transfers(&sources, 128, 1).unwrap()[0]
            .indexed
            .index,
        0,
        "a full-size budget must still consider the earliest missing chunk"
    );
}
