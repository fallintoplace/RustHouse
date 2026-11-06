use super::*;

#[test]
fn direct_runs_match_materialize_then_coalesce_over_sparse_checkpoints() {
    for bytes in [0u64, 1, 7, 8, 9, 63, 65, 4097, 65_536] {
        let count = bytes.div_ceil(7) as usize;
        for stride in [1usize, 2, 3, 13, 31] {
            let mut progress = crate::distributed_resume_map::ResumeMap::new(count).unwrap();
            for index in (0..count).step_by(stride) {
                progress.mark_complete(index).unwrap();
            }
            for budget in [0, 1, 6, 7, 13, 21, 1000] {
                for limit in [0, 1, 2, 7, 128] {
                    for max_run in [1, 6, 7, 14, 63, 8192] {
                        let reference = plan_missing_chunks_from_resume(
                            bytes, 7, 65_536, &progress, budget, limit,
                        )
                        .and_then(|chunks| coalesce_indexed_chunks(&chunks, max_run));
                        let planned = plan_missing_runs_from_resume(
                            bytes, 7, 65_536, &progress, budget, limit, max_run,
                        );
                        assert_eq!(
                            planned, reference,
                            "{bytes}:{stride}:{budget}:{limit}:{max_run}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn rejects_invalid_budgets_and_layouts_without_partial_runs() {
    let map = crate::distributed_resume_map::ResumeMap::new(2).unwrap();
    assert!(plan_missing_runs_from_resume(10, 4, 3, &map, 8, 2, 8).is_err());
    assert!(plan_missing_runs_from_resume(8, 4, 3, &map, 8, 2, 0).is_err());
    assert!(plan_missing_runs_from_resume(8, 4, 3, &map, 8, 65_537, 8).is_err());
    assert_eq!(
        plan_missing_runs_from_resume(8, 4, 3, &map, 8, 2, 8).unwrap(),
        vec![ChunkRun {
            first_index: 0,
            chunk_count: 2,
            offset: 0,
            length: 8
        }]
    );
}
