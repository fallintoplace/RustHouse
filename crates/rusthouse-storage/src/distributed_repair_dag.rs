//! Deterministic bounded dependency scheduling for offline replica repair.
//! The planner does not execute transfers or claim distributed coordination.
use std::collections::{HashMap, HashSet};

const MAX_TASKS: usize = 1024;
const MAX_DEPENDENCIES: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepairTask {
    pub id: String,
    pub bytes: u64,
    pub depends_on: Vec<String>,
}

/// A wave contains mutually independent tasks; later waves may depend on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepairWave {
    pub task_ids: Vec<String>,
    pub bytes: u64,
}

pub fn plan_repair_waves(
    tasks: &[RepairTask],
    max_parallel: usize,
    max_wave_bytes: u64,
) -> Result<Vec<RepairWave>, &'static str> {
    if tasks.len() > MAX_TASKS || max_parallel == 0 || max_parallel > MAX_TASKS
        || max_wave_bytes == 0
    {
        return Err("invalid repair scheduler bounds");
    }
    let mut by_id = HashMap::with_capacity(tasks.len());
    for (index, task) in tasks.iter().enumerate() {
        if task.id.is_empty() || task.id.len() > 64
            || !task.id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            || task.bytes == 0 || task.bytes > max_wave_bytes
            || by_id.insert(task.id.as_str(), index).is_some()
        {
            return Err("invalid repair task identity or size");
        }
    }
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); tasks.len()];
    let mut indegree = vec![0usize; tasks.len()];
    let mut total_edges = 0usize;
    for (index, task) in tasks.iter().enumerate() {
        let mut seen = HashSet::new();
        for dep in &task.depends_on {
            let parent = *by_id.get(dep.as_str()).ok_or("unknown repair prerequisite")?;
            if parent == index || !seen.insert(parent) {
                return Err("self-dependent or repeated repair prerequisite");
            }
            total_edges += 1;
            if total_edges > MAX_DEPENDENCIES {
                return Err("repair dependency graph exceeds limit");
            }
            edges[parent].push(index);
            indegree[index] += 1;
        }
    }

    let mut completed = vec![false; tasks.len()];
    let mut count = 0usize;
    let mut waves = Vec::new();
    while count < tasks.len() {
        let mut ready: Vec<usize> = (0..tasks.len())
            .filter(|&index| !completed[index] && indegree[index] == 0)
            .collect();
        if ready.is_empty() {
            return Err("cycle in replica repair dependencies");
        }
        ready.sort_unstable_by(|&a, &b| {
            tasks[b].bytes.cmp(&tasks[a].bytes).then_with(|| tasks[a].id.cmp(&tasks[b].id))
        });
        let mut chosen = Vec::new();
        let mut remaining = max_wave_bytes;
        for index in ready {
            if chosen.len() == max_parallel { break; }
            if tasks[index].bytes <= remaining {
                remaining -= tasks[index].bytes;
                chosen.push(index);
            }
        }
        if chosen.is_empty() {
            return Err("repair task exceeds remaining wave budget");
        }
        let bytes = max_wave_bytes - remaining;
        let task_ids = chosen.iter().map(|&index| tasks[index].id.clone()).collect();
        for index in chosen {
            completed[index] = true;
            count += 1;
            for &child in &edges[index] {
                indegree[child] -= 1;
            }
        }
        waves.push(RepairWave { task_ids, bytes });
    }
    Ok(waves)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, bytes: u64, deps: &[&str]) -> RepairTask {
        RepairTask { id: id.into(), bytes, depends_on: deps.iter().map(|s| (*s).into()).collect() }
    }

    #[test]
    fn schedules_ready_tasks_under_parallel_and_byte_bounds() {
        let tasks = [
            task("publish", 2, &["transfer", "verify"]),
            task("verify", 3, &["transfer"]),
            task("other", 2, &[]),
            task("transfer", 6, &[]),
        ];
        let waves = plan_repair_waves(&tasks, 2, 8).unwrap();
        assert_eq!(waves[0].task_ids, ["transfer", "other"]);
        assert_eq!(waves[0].bytes, 8);
        assert_eq!(waves[1].task_ids, ["verify"]);
        assert_eq!(waves[2].task_ids, ["publish"]);
    }

    #[test]
    fn rejects_cycles_and_bad_dependencies() {
        assert!(plan_repair_waves(&[task("a", 1, &["b"]), task("b", 1, &["a"])], 2, 10).is_err());
        assert!(plan_repair_waves(&[task("a", 1, &["missing"])], 2, 10).is_err());
        assert!(plan_repair_waves(&[task("a", 1, &["a"])], 2, 10).is_err());
        assert!(plan_repair_waves(&[task("a", 1, &["b", "b"]), task("b", 1, &[])], 2, 10).is_err());
    }

    #[test]
    fn task_larger_than_budget_fails_without_partial_plan() {
        assert!(plan_repair_waves(&[task("a", 11, &[])], 1, 10).is_err());
        assert!(plan_repair_waves(&[task("a", 1, &[])], 0, 10).is_err());
        assert_eq!(plan_repair_waves(&[], 1, 10).unwrap(), vec![]);
    }
}
