//! Bounded under-replication repair planning without network side effects.
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairJob {
    pub shard: String,
    pub bytes: u64,
    pub missing_replicas: u8,
}

fn valid_repair_job(job: &RepairJob) -> bool {
    !job.shard.is_empty()
        && job.shard.len() <= 128
        && job.missing_replicas != 0
        && job
            .shard
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn repair_priority(left: &RepairJob, right: &RepairJob) -> std::cmp::Ordering {
    right
        .missing_replicas
        .cmp(&left.missing_replicas)
        .then_with(|| left.bytes.cmp(&right.bytes))
        .then_with(|| left.shard.cmp(&right.shard))
}

/// Prioritize severe under-replication, then smaller objects, then shard ID.
/// The result is a schedule only; a transfer engine must execute it.
pub fn plan_repairs(
    jobs: &[RepairJob],
    byte_budget: u64,
    job_limit: usize,
) -> Result<Vec<RepairJob>, &'static str> {
    if jobs.len() > 4096 || job_limit > 4096 {
        return Err("repair job count exceeds bound");
    }
    let mut unique = HashSet::with_capacity(jobs.len());
    for job in jobs {
        if !valid_repair_job(job) || !unique.insert(job.shard.as_str())
        {
            return Err("invalid or duplicate repair job");
        }
    }
    if job_limit == 0 {
        return Ok(Vec::new());
    }
    let mut ranked: Vec<&RepairJob> = jobs.iter().collect();
    ranked.sort_unstable_by(|left, right| repair_priority(left, right));
    let mut selected = Vec::with_capacity(jobs.len().min(job_limit));
    let mut remaining = byte_budget;
    for job in ranked {
        if selected.len() == job_limit {
            break;
        }
        if job.bytes <= remaining {
            remaining -= job.bytes;
            selected.push((*job).clone());
        }
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_urgent_repair_without_exceeding_budget() {
        let jobs = vec![
            RepairJob { shard: "a".into(), bytes: 40, missing_replicas: 1 },
            RepairJob { shard: "b".into(), bytes: 60, missing_replicas: 3 },
            RepairJob { shard: "c".into(), bytes: 20, missing_replicas: 2 },
        ];
        let plan = plan_repairs(&jobs, 80, 2).unwrap();
        assert_eq!(plan.iter().map(|job| job.shard.as_str()).collect::<Vec<_>>(), vec!["b", "c"]);
    }

    #[test]
    fn rejects_ambiguous_jobs() {
        let job = RepairJob { shard: "a".into(), bytes: 1, missing_replicas: 1 };
        assert!(plan_repairs(&[job.clone(), job], 10, 2).is_err());
    }
    #[test]
    fn skips_unschedulable_jobs_without_losing_lower_priority_work() {
        let jobs = vec![
            RepairJob { shard: "urgent".into(), bytes: 100, missing_replicas: 3 },
            RepairJob { shard: "small".into(), bytes: 1, missing_replicas: 2 },
            RepairJob { shard: "extra".into(), bytes: 1, missing_replicas: 1 },
        ];
        let plan = plan_repairs(&jobs, 1, 1).unwrap();
        assert_eq!(plan[0].shard, "small");
        assert_eq!(plan.len(), 1);
        assert!(plan_repairs(&jobs, 1, 0).unwrap().is_empty());
    }

}