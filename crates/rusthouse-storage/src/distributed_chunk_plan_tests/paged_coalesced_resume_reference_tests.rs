use super::*;
use crate::distributed_resume_map::ResumeMap;

#[test]
fn paged_run_planning_matches_materialize_then_coalesce() {
    for total_bytes in [0u64, 1, 7, 8, 9, 63, 64, 65, 127, 128, 129, 2049] {
        let count = total_bytes.div_ceil(7) as usize;
        let mut progress = ResumeMap::new(count).unwrap();
        for index in 0..count {
            if index % 5 == 0 || index % 13 == 0 {
                progress.mark_complete(index).unwrap();
            }
        }
        for cursor in 0..=count {
            for budget in [0, 1, 6, 7, 10, 21, 300] {
                for limit in [0, 1, 2, 5, 32] {
                    for max_run in [1, 7, 14, 100] {
                        let expected = plan_missing_chunks_from_resume_after(
                            total_bytes,
                            7,
                            65_536,
                            &progress,
                            cursor,
                            budget,
                            limit,
                        )
                        .and_then(|chunks| coalesce_indexed_chunks(&chunks, max_run));
                        let actual = plan_missing_runs_from_resume_after(
                            total_bytes,
                            7,
                            65_536,
                            &progress,
                            cursor,
                            budget,
                            limit,
                            max_run,
                        );
                        assert_eq!(
                            actual, expected,
                            "bytes={total_bytes} cursor={cursor} budget={budget} limit={limit} max_run={max_run}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn rejects_invalid_cursors_even_for_empty_or_zero_budget_pages() {
    let progress = ResumeMap::new(2).unwrap();
    assert!(plan_missing_runs_from_resume_after(8, 4, 2, &progress, 3, 0, 0, 8).is_err());
    assert!(plan_missing_runs_from_resume_after(8, 4, 2, &progress, 2, 0, 0, 0).is_err());
    assert!(plan_missing_runs_from_resume_after(10, 4, 3, &progress, 0, 0, 0, 8).is_err());
    assert!(
        plan_missing_runs_from_resume_after(8, 4, 2, &progress, 2, 8, 4, 8)
            .unwrap()
            .is_empty()
    );
}
