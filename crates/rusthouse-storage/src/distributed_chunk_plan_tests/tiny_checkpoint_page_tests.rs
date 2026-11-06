use super::*;
use crate::distributed_resume_map::ResumeMap;

#[test]
fn tiny_pages_keep_order_and_budget_for_dense_and_sparse_checkpoints() {
    let chunks = 65_536;
    let mut progress = ResumeMap::new(chunks).unwrap();
    for index in 0..chunks {
        if index != 17 && index != 33_333 && index != 65_535 {
            progress.mark_complete(index).unwrap();
        }
    }
    for page_limit in [1, 2, 3, 4, 8, 64] {
        for byte_budget in [1, 2, 4, 64, u64::MAX] {
            let expected = plan_missing_chunks(
                chunks as u64,
                1,
                chunks,
                &progress.remaining(),
                byte_budget,
                page_limit,
            )
            .unwrap();
            let actual = plan_missing_chunks_from_resume(
                chunks as u64,
                1,
                chunks,
                &progress,
                byte_budget,
                page_limit,
            )
            .unwrap();
            assert_eq!(
                actual, expected,
                "page_limit={page_limit} budget={byte_budget}"
            );
        }
    }
}
