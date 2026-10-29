//! Bounded under-replication repair planning without network side effects.
use std::collections::HashSet;

const MAX_REPAIR_JOBS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairJob {
    pub shard: String,
    pub bytes: u64,
    pub missing_replicas: u8,
}

fn valid_shard(shard: &str) -> bool {
    !shard.is_empty()
        && shard.len() <= 128
        && shard
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_repair_job(job: &RepairJob) -> bool {
    job.missing_replicas != 0 && valid_shard(&job.shard)
}

fn repair_priority(left: &RepairJob, right: &RepairJob) -> std::cmp::Ordering {
    right
        .missing_replicas
        .cmp(&left.missing_replicas)
        .then_with(|| left.bytes.cmp(&right.bytes))
        .then_with(|| left.shard.cmp(&right.shard))
}

fn validate_repair_jobs(jobs: &[RepairJob]) -> Result<(), &'static str> {
    let mut unique = HashSet::with_capacity(jobs.len());
    for job in jobs {
        if !valid_repair_job(job) || !unique.insert(job.shard.as_str()) {
            return Err("invalid or duplicate repair job");
        }
    }
    Ok(())
}

// Work with borrowed jobs until the scheduler knows which ones fit.
// This avoids cloning every pending job when only a small batch is selected.
fn plan_ranked_repairs<'a>(
    jobs: impl Iterator<Item = &'a RepairJob>,
    byte_budget: u64,
    job_limit: usize,
) -> Vec<RepairJob> {
    if job_limit == 0 {
        return Vec::new();
    }
    let eligible = jobs.filter(|job| job.bytes <= byte_budget);
    if job_limit == 1 {
        return eligible
            .min_by(|left, right| repair_priority(left, right))
            .cloned()
            .into_iter()
            .collect();
    }
    let mut ranked: Vec<&RepairJob> = eligible.collect();
    ranked.sort_unstable_by(|left, right| repair_priority(left, right));
    let mut selected = Vec::with_capacity(ranked.len().min(job_limit));
    let mut remaining = byte_budget;
    for job in ranked {
        if selected.len() == job_limit {
            break;
        }
        if job.bytes <= remaining {
            remaining -= job.bytes;
            selected.push(job.clone());
        }
    }
    selected
}

/// Prioritize severe under-replication, then smaller objects, then shard ID.
/// The result is a schedule only; a transfer engine must execute it.
pub fn plan_repairs(
    jobs: &[RepairJob],
    byte_budget: u64,
    job_limit: usize,
) -> Result<Vec<RepairJob>, &'static str> {
    if jobs.len() > MAX_REPAIR_JOBS || job_limit > MAX_REPAIR_JOBS {
        return Err("repair job count exceeds bound");
    }
    validate_repair_jobs(jobs)?;
    Ok(plan_ranked_repairs(jobs.iter(), byte_budget, job_limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_urgent_repair_without_exceeding_budget() {
        let jobs = vec![
            RepairJob {
                shard: "a".into(),
                bytes: 40,
                missing_replicas: 1,
            },
            RepairJob {
                shard: "b".into(),
                bytes: 60,
                missing_replicas: 3,
            },
            RepairJob {
                shard: "c".into(),
                bytes: 20,
                missing_replicas: 2,
            },
        ];
        let plan = plan_repairs(&jobs, 80, 2).unwrap();
        assert_eq!(
            plan.iter()
                .map(|job| job.shard.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "c"]
        );
    }

    #[test]
    fn rejects_ambiguous_jobs() {
        let job = RepairJob {
            shard: "a".into(),
            bytes: 1,
            missing_replicas: 1,
        };
        assert!(plan_repairs(&[job.clone(), job], 10, 2).is_err());
    }
    #[test]
    fn skips_unschedulable_jobs_without_losing_lower_priority_work() {
        let jobs = vec![
            RepairJob {
                shard: "urgent".into(),
                bytes: 100,
                missing_replicas: 3,
            },
            RepairJob {
                shard: "small".into(),
                bytes: 1,
                missing_replicas: 2,
            },
            RepairJob {
                shard: "extra".into(),
                bytes: 1,
                missing_replicas: 1,
            },
        ];
        let plan = plan_repairs(&jobs, 1, 1).unwrap();
        assert_eq!(plan[0].shard, "small");
        assert_eq!(plan.len(), 1);
        assert!(plan_repairs(&jobs, 1, 0).unwrap().is_empty());
    }
}

/// Do not schedule repairs for shards already assigned to an active worker.
/// In-flight identifiers are trusted scheduler state, not completion claims.
pub fn plan_repairs_excluding(
    jobs: &[RepairJob],
    in_flight: &[&str],
    byte_budget: u64,
    job_limit: usize,
) -> Result<Vec<RepairJob>, &'static str> {
    if jobs.len() > MAX_REPAIR_JOBS || job_limit > MAX_REPAIR_JOBS {
        return Err("repair job count exceeds bound");
    }
    if in_flight.len() > MAX_REPAIR_JOBS {
        return Err("too many in-flight repair shards");
    }
    validate_repair_jobs(jobs)?;
    let mut excluded = HashSet::with_capacity(in_flight.len());
    for &shard in in_flight {
        if !valid_shard(shard) || !excluded.insert(shard) {
            return Err("invalid or duplicate in-flight repair shard");
        }
    }
    Ok(plan_ranked_repairs(
        jobs.iter().filter(|job| !excluded.contains(job.shard.as_str())),
        byte_budget,
        job_limit,
    ))
}

#[cfg(test)]
mod in_flight_repair_tests {
    use super::*;

    #[test]
    fn borrowed_plan_matches_full_plan_when_nothing_is_in_flight() {
        let jobs = (0..1024)
            .map(|index| RepairJob {
                shard: format!("shard_{index:04}"),
                bytes: 1 + index % 17,
                missing_replicas: 1 + (index % 3) as u8,
            })
            .collect::<Vec<_>>();
        for limit in [0, 1, 2, 8, 100, 1024] {
            for budget in [0, 1, 13, 128, 1024] {
                assert_eq!(
                    plan_repairs_excluding(&jobs, &[], budget, limit).unwrap(),
                    plan_repairs(&jobs, budget, limit).unwrap(),
                );
            }
        }
        let selected = plan_repairs_excluding(&jobs, &["shard_0001"], 256, 3).unwrap();
        assert!(selected.iter().all(|job| job.shard != "shard_0001"));
        assert!(plan_repairs_excluding(&jobs, &[], 100, MAX_REPAIR_JOBS + 1).is_err());
    }

    #[test]
    fn avoids_duplicate_urgent_repairs() {
        let jobs = [
            RepairJob { shard: "urgent".into(), bytes: 20, missing_replicas: 3 },
            RepairJob { shard: "pending".into(), bytes: 10, missing_replicas: 2 },
        ];
        let selected = plan_repairs_excluding(&jobs, &["urgent"], 20, 2).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].shard, "pending");
        assert!(plan_repairs_excluding(&jobs, &["urgent", "urgent"], 20, 2).is_err());
        assert!(plan_repairs_excluding(&jobs, &["../bad"], 20, 2).is_err());
    }
}

/// Plan repairs with an explicit replication factor. Reject impossible deficits.
pub fn plan_repairs_for_replication_factor(
    jobs: &[RepairJob],
    replication_factor: u8,
    byte_budget: u64,
    job_limit: usize,
) -> Result<Vec<RepairJob>, &'static str> {
    if replication_factor == 0 || replication_factor > 16 {
        return Err("invalid replica count for repair planning");
    }
    if jobs.iter().any(|job| job.missing_replicas > replication_factor) {
        return Err("repair deficit exceeds replication factor");
    }
    plan_repairs(jobs, byte_budget, job_limit)
}

#[cfg(test)]
mod replication_factor_tests {
    use super::*;

    #[test]
    fn rejects_impossible_replica_deficits_before_scheduling() {
        let jobs = [RepairJob { shard: "a".into(), bytes: 10, missing_replicas: 3 }];
        assert!(plan_repairs_for_replication_factor(&jobs, 2, 10, 1).is_err());
        assert!(plan_repairs_for_replication_factor(&jobs, 0, 10, 1).is_err());
        assert!(plan_repairs_for_replication_factor(&jobs, 17, 10, 1).is_err());
        assert_eq!(plan_repairs_for_replication_factor(&jobs, 3, 10, 1).unwrap(), jobs.to_vec());
    }
}
