//! Exact minimum-cost matching of repair tasks to distinct workers.
//! This chooses workers only; the caller remains responsible for I/O and locks.
const MAX_WORKERS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepairAssignment {
    /// Worker index for each input task, in input task order.
    pub workers: Vec<usize>,
    pub total_cost: u64,
}

fn reconstruct_workers(
    mut mask: usize,
    predecessor: &[Option<(usize, usize)>],
    task_count: usize,
) -> Result<Vec<usize>, &'static str> {
    let mut chosen = Vec::with_capacity(task_count);
    while mask != 0 {
        let (previous, worker) = predecessor[mask].ok_or("broken repair assignment path")?;
        chosen.push(worker);
        mask = previous;
    }
    chosen.reverse();
    Ok(chosen)
}

fn consider_assignment(
    mask: usize,
    worker: usize,
    current: u64,
    cost: Option<u64>,
    best: &mut [Option<u64>],
    predecessor: &mut [Option<(usize, usize)>],
) {
    let worker_bit = 1usize << worker;
    if mask & worker_bit != 0 {
        return;
    }
    let Some(cost) = cost else {
        return;
    };
    let Some(next_cost) = current.checked_add(cost) else {
        return;
    };
    let next = mask | worker_bit;
    if best[next].is_none_or(|existing| next_cost < existing) {
        best[next] = Some(next_cost);
        predecessor[next] = Some((mask, worker));
    }
}

fn validate_matrix(costs: &[Vec<Option<u64>>]) -> Result<usize, &'static str> {
    let workers = costs[0].len();
    if workers == 0
        || workers > MAX_WORKERS
        || costs.len() > workers
        || costs.iter().any(|row| row.len() != workers)
    {
        return Err("invalid repair assignment matrix");
    }
    Ok(workers)
}

/// None forbids an assignment. A worker can serve at most one task.
/// Runs in O(workers * 2^workers) memory and O(tasks * workers * 2^workers)
/// time, with an explicit 16-worker bound.
pub fn minimum_cost_assignment(
    costs: &[Vec<Option<u64>>],
) -> Result<RepairAssignment, &'static str> {
    if costs.is_empty() {
        return Ok(RepairAssignment {
            workers: Vec::new(),
            total_cost: 0,
        });
    }
    let workers = validate_matrix(costs)?;
    let states = 1usize << workers;
    let mut best = vec![None::<u64>; states];
    let mut predecessor = vec![None::<(usize, usize)>; states];
    best[0] = Some(0);
    for mask in 0..states {
        let task = mask.count_ones() as usize;
        if task >= costs.len() {
            continue;
        }
        let Some(current) = best[mask] else {
            continue;
        };
        for (worker, &cost) in costs[task].iter().enumerate() {
            consider_assignment(mask, worker, current, cost, &mut best, &mut predecessor);
        }
    }
    let (mask, cost) = (0..states)
        .filter(|&mask| mask.count_ones() as usize == costs.len())
        .filter_map(|mask| best[mask].map(|cost| (mask, cost)))
        .min_by_key(|&(mask, cost)| (cost, mask))
        .ok_or("no feasible bounded repair assignment")?;
    let chosen = reconstruct_workers(mask, &predecessor, costs.len())?;
    Ok(RepairAssignment {
        workers: chosen,
        total_cost: cost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_matching_beats_local_greedy_assignment() {
        let costs = vec![vec![Some(1), Some(2)], vec![Some(2), Some(100)]];
        let result = minimum_cost_assignment(&costs).unwrap();
        assert_eq!(result.workers, [1, 0]);
        assert_eq!(result.total_cost, 4);
    }

    #[test]
    fn respects_forbidden_edges_and_unassigned_workers() {
        let costs = vec![vec![None, Some(9), Some(1)], vec![Some(2), None, None]];
        let result = minimum_cost_assignment(&costs).unwrap();
        assert_eq!(result.workers, [2, 0]);
        assert_eq!(result.total_cost, 3);
        assert!(minimum_cost_assignment(&[vec![None]]).is_err());
    }

    #[test]
    fn reports_overflow_infeasibility_without_wrapping() {
        let costs = vec![vec![Some(u64::MAX), None], vec![None, Some(1)]];
        assert!(minimum_cost_assignment(&costs).is_err());
        assert!(minimum_cost_assignment(&[vec![Some(1)], vec![Some(2)]]).is_err());
        assert_eq!(minimum_cost_assignment(&[]).unwrap().total_cost, 0);
    }
}
