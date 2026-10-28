//! Bounded under-replication repair planning without network side effects.
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairJob {
    pub shard: String,
    pub bytes: u64,
    pub missing_replicas: u8,
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
    let mut unique = HashSet::new();
    for job in jobs {
        if job.shard.is_empty()
            || job.shard.len() > 128
            || job.missing_replicas == 0
            || !job
                .shard
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            || !unique.insert(job.shard.as_str())
        {
            return Err("invalid or duplicate repair job");
        }
    }
    let mut ranked = jobs.to_vec();
    ranked.sort_unstable_by(|left, right| {
        right
            .missing_replicas
            .cmp(&left.missing_replicas)
            .then_with(|| left.bytes.cmp(&right.bytes))
            .then_with(|| left.shard.cmp(&right.shard))
    });
    let mut selected = Vec::new();
    let mut remaining = byte_budget;
    for job in ranked {
        if selected.len() == job_limit {
            break;
        }
        if job.bytes <= remaining {
            remaining -= job.bytes;
            selected.push(job);
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
}