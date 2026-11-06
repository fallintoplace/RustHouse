use super::*;
use crate::distributed_object_key::ObjectKey;
use crate::distributed_resume_map::ResumeMap;

fn key(name: &str) -> ObjectKey {
    ObjectKey::new("cluster", "shard", 1, "part", name).unwrap()
}

#[test]
fn alternates_objects_and_respects_the_shared_byte_budget() {
    let keys = [key("a.bin"), key("b.bin")];
    let first = ResumeMap::new(6).unwrap();
    let mut second = ResumeMap::new(6).unwrap();
    second.mark_complete_indices(&[0, 2]).unwrap();
    let sources = [
        FairTransferSource {
            key: &keys[0],
            total_bytes: 6,
            chunk_bytes: 1,
            checkpoint: &first,
        },
        FairTransferSource {
            key: &keys[1],
            total_bytes: 6,
            chunk_bytes: 1,
            checkpoint: &second,
        },
    ];
    let selected = plan_fair_replica_transfers(&sources, 6, 6).unwrap();
    let pairs = selected
        .iter()
        .map(|item| (item.source_index, item.indexed.index))
        .collect::<Vec<_>>();
    assert_eq!(pairs, [(0, 0), (1, 1), (0, 1), (1, 3), (0, 2), (1, 4)]);
    assert_eq!(
        selected
            .iter()
            .map(|item| item.indexed.chunk.length)
            .sum::<u64>(),
        6
    );
    assert_eq!(first.completed_count(), 0);
    assert_eq!(second.completed_count(), 2);
}

#[test]
fn short_tail_is_not_starved_by_full_unaffordable_chunks() {
    let keys = [key("large.bin"), key("tail.bin")];
    let full = ResumeMap::new(3).unwrap();
    let tail = ResumeMap::new(3).unwrap();
    let sources = [
        FairTransferSource {
            key: &keys[0],
            total_bytes: 24,
            chunk_bytes: 8,
            checkpoint: &full,
        },
        FairTransferSource {
            key: &keys[1],
            total_bytes: 17,
            chunk_bytes: 8,
            checkpoint: &tail,
        },
    ];
    assert_eq!(
        plan_fair_replica_transfers(&sources, 1, 8).unwrap(),
        [FairTransferChunk {
            source_index: 1,
            indexed: IndexedChunk {
                index: 2,
                chunk: Chunk {
                    offset: 16,
                    length: 1
                }
            },
        }],
    );
}

#[test]
fn small_budget_fast_rejects_large_complete_chunks_but_keeps_short_tails() {
    let keys = (0..64)
        .map(|index| key(&format!("file{index}.bin")))
        .collect::<Vec<_>>();
    let checkpoints = (0..64)
        .map(|_| ResumeMap::new(65_536).unwrap())
        .collect::<Vec<_>>();
    let sources = keys
        .iter()
        .zip(&checkpoints)
        .map(|(key, checkpoint)| FairTransferSource {
            key,
            total_bytes: 65_536 * 8,
            chunk_bytes: 8,
            checkpoint,
        })
        .collect::<Vec<_>>();
    assert!(
        plan_fair_replica_transfers(&sources, 1, 64)
            .unwrap()
            .is_empty()
    );

    let short_tail = ResumeMap::new(2).unwrap();
    let tail_key = key("short.bin");
    let only_tail = [FairTransferSource {
        key: &tail_key,
        total_bytes: 9,
        chunk_bytes: 8,
        checkpoint: &short_tail,
    }];
    assert_eq!(
        plan_fair_replica_transfers(&only_tail, 1, 2).unwrap()[0]
            .indexed
            .index,
        1
    );
}

#[test]
fn rejects_bad_layouts_and_duplicate_objects_even_with_zero_budget() {
    let id = key("one.bin");
    let valid = ResumeMap::new(2).unwrap();
    let bad = ResumeMap::new(1).unwrap();
    let source = FairTransferSource {
        key: &id,
        total_bytes: 8,
        chunk_bytes: 4,
        checkpoint: &valid,
    };
    let duplicate = [
        FairTransferSource { ..source },
        FairTransferSource { ..source },
    ];
    assert!(plan_fair_replica_transfers(&duplicate, 0, 0).is_err());
    assert!(
        plan_fair_replica_transfers(
            &[FairTransferSource {
                checkpoint: &bad,
                ..source
            },],
            0,
            0
        )
        .is_err()
    );
    assert!(plan_fair_replica_transfers(&[source], 1, 65_537).is_err());
}
