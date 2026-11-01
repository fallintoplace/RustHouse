//! Checked storage-capacity accounting for planned object transfers.

const MAX_RESERVATION_OBJECTS: usize = 65_536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacity {
    pub total: u64,
    pub used: u64,
    pub reserved: u64,
}

impl Capacity {
    pub fn available(self) -> Result<u64, &'static str> {
        let committed = self
            .used
            .checked_add(self.reserved)
            .ok_or("replica capacity overflow")?;
        self.total
            .checked_sub(committed)
            .ok_or("replica capacity exceeded")
    }

    fn remaining_reservation(
        self,
        bytes: u64,
        error: &'static str,
    ) -> Result<u64, &'static str> {
        self.reserved.checked_sub(bytes).ok_or(error)
    }

    fn add_reservation(self, bytes: u64) -> Result<Self, &'static str> {
        Ok(Self {
            reserved: self
                .reserved
                .checked_add(bytes)
                .ok_or("replica reservation overflow")?,
            ..self
        })
    }

    /// Reserve bytes without changing the used-byte count.
    pub fn reserve(self, bytes: u64) -> Result<Self, &'static str> {
        if bytes > self.available()? {
            return Err("insufficient replica capacity");
        }
        self.add_reservation(bytes)
    }

    /// Convert reserved bytes into committed usage.
    pub fn commit(self, bytes: u64) -> Result<Self, &'static str> {
        let remaining_reserved_bytes = self.remaining_reservation(bytes, "cannot commit unreserved bytes")?;
        self.available()?;
        Ok(Self {
            used: self
                .used
                .checked_add(bytes)
                .ok_or("replica usage overflow")?,
            reserved: remaining_reserved_bytes,
            ..self
        })
    }

    pub fn release(self, bytes: u64) -> Result<Self, &'static str> {
        let released = Self {
            reserved: self.remaining_reservation(bytes, "cannot release unreserved bytes")?,
            ..self
        };
        released.available()?;
        Ok(released)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_lifecycle_preserves_total_capacity() {
        let initial = Capacity {
            total: 100,
            used: 30,
            reserved: 0,
        };
        let reserved = initial.reserve(50).unwrap();
        assert_eq!(reserved.available().unwrap(), 20);
        let committed = reserved.commit(20).unwrap();
        assert_eq!(committed.used, 50);
        assert_eq!(committed.reserved, 30);
        assert_eq!(committed.release(30).unwrap().available().unwrap(), 50);
    }

    #[test]
    fn refuses_to_commit_existing_overcommit() {
        let invalid = Capacity {
            total: 10,
            used: 9,
            reserved: 2,
        };
        assert!(invalid.commit(1).is_err());
        assert_eq!(invalid.release(1).unwrap().available().unwrap(), 0);
    }

    #[test]
    fn rejects_overcommit_and_underflow() {
        let cap = Capacity {
            total: 10,
            used: 9,
            reserved: 1,
        };
        assert!(cap.reserve(1).is_err());
        assert!(cap.commit(2).is_err());
        assert!(
            Capacity {
                total: 1,
                used: u64::MAX,
                reserved: 2
            }
            .available()
            .is_err()
        );
    }
    #[test]
    fn small_capacity_states_preserve_accounting_invariants() {
        for total in 0..=12u64 {
            for used in 0..=total {
                for reserved in 0..=total - used {
                    let state = Capacity {
                        total,
                        used,
                        reserved,
                    };
                    let available = state.available().unwrap();
                    assert_eq!(used + reserved + available, total);
                    let held = state.reserve(available).unwrap();
                    assert_eq!(held.available().unwrap(), 0);
                    let restored = held.release(available).unwrap();
                    assert_eq!(restored, state);
                    let committed = state.commit(reserved).unwrap();
                    assert_eq!(committed.used, used + reserved);
                    assert_eq!(committed.reserved, 0);
                }
            }
        }
    }
}

impl Capacity {
    /// Reserve a whole immutable-object batch or reject it without changing state.
    pub fn reserve_many(self, object_sizes: &[u64]) -> Result<Self, &'static str> {
        if object_sizes.len() > MAX_RESERVATION_OBJECTS {
            return Err("replica object batch exceeds limit");
        }
        let available = self.available()?;
        let mut bytes = 0u64;
        for &size in object_sizes {
            bytes = bytes.checked_add(size).ok_or("replica batch size overflow")?;
            if bytes > available {
                return Err("insufficient replica capacity");
            }
        }
        self.add_reservation(bytes)
    }
}

#[cfg(test)]
mod batch_reservation_tests {
    use super::*;

    #[test]
    fn reserves_all_objects_as_one_transaction() {
        let capacity = Capacity { total: 100, used: 10, reserved: 5 };
        let booked = capacity.reserve_many(&[30, 25, 30]).unwrap();
        assert_eq!(booked.available().unwrap(), 0);
        assert_eq!(booked.reserved, 90);
        assert!(capacity.reserve_many(&[30, 25, 31]).is_err());
        assert_eq!(capacity.reserved, 5);
    }

    #[test]
    fn stops_reserving_when_a_batch_already_exceeds_capacity() {
        let capacity = Capacity { total: 10, used: 1, reserved: 2 };
        assert_eq!(capacity.reserve_many(&[3, 4]).unwrap().available().unwrap(), 0);
        assert!(capacity.reserve_many(&[3, 5, u64::MAX]).is_err());
        assert_eq!(capacity.reserve_many(&[0, 0, 0]).unwrap(), capacity);
        assert_eq!(capacity, Capacity { total: 10, used: 1, reserved: 2 });
    }

    #[test]
    fn refuses_overflow_and_unbounded_batches() {
        let capacity = Capacity { total: u64::MAX, used: 0, reserved: 0 };
        assert!(capacity.reserve_many(&[u64::MAX, 1]).is_err());
        assert!(capacity.reserve_many(&vec![0; 65_537]).is_err());
        assert_eq!(capacity.reserve_many(&[]).unwrap(), capacity);
    }
}

impl Capacity {
    /// Release committed storage usage after verified object deletion.
    /// The caller is responsible for ensuring the physical deletion succeeded.
    pub fn reclaim_used(self, bytes: u64) -> Result<Self, &'static str> {
        self.available()?;
        Ok(Self {
            used: self.used.checked_sub(bytes).ok_or("cannot reclaim unused bytes")?,
            ..self
        })
    }

    /// Apply a verified batch of object deletions as one capacity transition.
    /// Invalid original states, oversized batches and arithmetic overflows are
    /// rejected before returning a new accounting state.
    pub fn reclaim_used_many(self, object_sizes: &[u64]) -> Result<Self, &'static str> {
        if object_sizes.len() > MAX_RESERVATION_OBJECTS {
            return Err("replica object batch exceeds limit");
        }
        self.available()?;
        let reclaimed = object_sizes.iter().try_fold(0u64, |total, &bytes| {
            total.checked_add(bytes).ok_or("replica batch size overflow")
        })?;
        self.reclaim_used(reclaimed)
    }
}

#[cfg(test)]
mod reclaim_used_tests {
    use super::*;

    #[test]
    fn reclaim_frees_only_committed_bytes_and_preserves_reservations() {
        let initial = Capacity { total: 100, used: 60, reserved: 20 };
        let after = initial.reclaim_used(30).unwrap();
        assert_eq!(after, Capacity { total: 100, used: 30, reserved: 20 });
        assert_eq!(after.available().unwrap(), 50);
        assert!(initial.reclaim_used(61).is_err());
        assert_eq!(initial.reclaim_used(0).unwrap(), initial);
        assert!(Capacity { total: 1, used: 2, reserved: 0 }.reclaim_used(1).is_err());
    }
}

#[cfg(test)]
mod batch_reservation_fast_path_tests {
    use super::*;

    #[test]
    fn reserved_bytes_match_single_reservation_for_every_valid_small_state() {
        for total in 0..=16u64 {
            for used in 0..=total {
                for reserved in 0..=total - used {
                    let capacity = Capacity { total, used, reserved };
                    for first in 0..=total - used - reserved {
                        let second = (total - used - reserved) - first;
                        assert_eq!(capacity.reserve_many(&[first, second]).unwrap(), capacity.reserve(first + second).unwrap());
                    }
                }
            }
        }
    }
}

impl Capacity {
    /// Commit a verified batch as one accounting transaction. The capacity
    /// value is unchanged on any invalid byte sum or insufficient reservation.
    pub fn commit_many(self, object_sizes: &[u64]) -> Result<Self, &'static str> {
        if object_sizes.len() > MAX_RESERVATION_OBJECTS {
            return Err("replica object batch exceeds limit");
        }
        let total = object_sizes.iter().try_fold(0u64, |sum, &size| {
            sum.checked_add(size).ok_or("replica batch size overflow")
        })?;
        self.commit(total)
    }

    /// Cancel a whole pending batch without allowing reservation underflow.
    pub fn release_many(self, object_sizes: &[u64]) -> Result<Self, &'static str> {
        if object_sizes.len() > MAX_RESERVATION_OBJECTS {
            return Err("replica object batch exceeds limit");
        }
        let total = object_sizes.iter().try_fold(0u64, |sum, &size| {
            sum.checked_add(size).ok_or("replica batch size overflow")
        })?;
        self.release(total)
    }
}

#[cfg(test)]
mod atomic_batch_capacity_tests {
    use super::*;

    #[test]
    fn batch_reserve_commit_release_preserve_exact_usage() {
        let initial = Capacity { total: 100, used: 9, reserved: 3 };
        let reserved = initial.reserve_many(&[11, 19, 7]).unwrap();
        let committed = reserved.commit_many(&[11, 19]).unwrap();
        assert_eq!(committed, Capacity { total: 100, used: 39, reserved: 10 });
        assert_eq!(committed.release_many(&[7]).unwrap(),
            Capacity { total: 100, used: 39, reserved: 3 });
        assert!(reserved.commit_many(&[11, 19, 60]).is_err());
        assert!(reserved.release_many(&[11, 19, 60]).is_err());
        assert_eq!(initial.reserve_many(&[11, 19, 7]).unwrap(), reserved);
    }

    #[test]
    fn checked_batched_transitions_match_sequential_for_valid_states() {
        for total in 0..=24u64 {
            for used in 0..=total {
                for held in 0..=total - used {
                    let state = Capacity { total, used, reserved: held };
                    for first in 0..=held {
                        for second in 0..=held - first {
                            assert_eq!(
                                state.commit_many(&[first, second]).unwrap(),
                                state.commit(first).unwrap().commit(second).unwrap(),
                            );
                            assert_eq!(
                                state.release_many(&[first, second]).unwrap(),
                                state.release(first).unwrap().release(second).unwrap(),
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn batch_totals_reject_overflow_and_oversized_lists() {
        let state = Capacity { total: u64::MAX, used: 0, reserved: u64::MAX };
        assert!(state.commit_many(&[u64::MAX, 1]).is_err());
        assert!(state.release_many(&[u64::MAX, 1]).is_err());
        assert!(state.commit_many(&vec![0; 65_537]).is_err());
        assert!(state.release_many(&vec![0; 65_537]).is_err());
        assert_eq!(state.commit_many(&[]).unwrap(), state);
    }
}

#[cfg(test)]
mod atomic_committed_reclaim_tests {
    use super::*;

    #[test]
    fn batch_deletions_preserve_reservations_and_capacity_invariants() {
        for total in 0..=20u64 {
            for used in 0..=total {
                for reserved in 0..=total - used {
                    let capacity = Capacity { total, used, reserved };
                    for first in 0..=used {
                        let second = used - first;
                        let expected = capacity.reclaim_used(first).unwrap()
                            .reclaim_used(second).unwrap();
                        let batched = capacity.reclaim_used_many(&[first, second]).unwrap();
                        assert_eq!(batched, expected);
                        assert_eq!(batched.reserved, reserved);
                        assert_eq!(batched.available().unwrap(), total - reserved);
                    }
                }
            }
        }
    }

    #[test]
    fn detects_overflow_unaccounted_deletion_and_bad_state() {
        let state = Capacity { total: 100, used: 50, reserved: 20 };
        assert_eq!(state.reclaim_used_many(&[20, 10]).unwrap(),
            Capacity { total: 100, used: 20, reserved: 20 });
        assert!(state.reclaim_used_many(&[30, 21]).is_err());
        assert!(state.reclaim_used_many(&[u64::MAX, 1]).is_err());
        assert!(state.reclaim_used_many(&vec![0; 65_537]).is_err());
        assert_eq!(state.reclaim_used_many(&[]).unwrap(), state);
        assert!(Capacity { total: 1, used: 2, reserved: 0 }
            .reclaim_used_many(&[]).is_err());
    }
}

/// One node's accounting state for a multi-replica transfer plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaNodeCapacity {
    pub node: String,
    pub capacity: Capacity,
}

/// Simulate all requested replica reservations before returning any new state.
///
/// Repeated requests for one node accumulate in input order. The returned
/// states preserve the input node ordering. This is atomic value planning,
/// not an actual distributed reservation or transaction.
pub fn reserve_replica_capacity_plan(
    nodes: &[ReplicaNodeCapacity],
    requests: &[(&str, u64)],
) -> Result<Vec<ReplicaNodeCapacity>, &'static str> {
    if nodes.is_empty() || nodes.len() > 64 || requests.len() > MAX_RESERVATION_OBJECTS {
        return Err("invalid multi-replica reservation limits");
    }
    // Keep small node lookups borrowed; large groups use the indexed path.
    let mut index = (nodes.len() > 8)
        .then(|| std::collections::HashMap::with_capacity(nodes.len()));
    let mut available = [0u64; 64];
    for (position, node) in nodes.iter().enumerate() {
        let duplicate = if let Some(index) = index.as_mut() {
            index.insert(node.node.as_str(), position).is_some()
        } else {
            nodes[..position].iter().any(|previous| previous.node == node.node)
        };
        if node.node.is_empty()
            || node.node.len() > 64
            || !node.node.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
            })
            || duplicate
        {
            return Err("invalid or duplicate reservation node");
        }
        available[position] = node.capacity.available()?;
    }

    // Aggregate on the stack before creating the owned result. In large
    // batches, thousands of requests may target one replica. Preserve
    // the first-overcommit error by checking each incremental total.
    let mut reserved = [0u64; 64];
    for &(node, bytes) in requests {
        let position = if let Some(index) = &index {
            *index.get(node).ok_or("unknown replica reservation node")?
        } else {
            nodes.iter().position(|entry| entry.node == node)
                .ok_or("unknown replica reservation node")?
        };
        let updated = reserved[position].checked_add(bytes)
            .ok_or("insufficient replica capacity")?;
        if updated > available[position] {
            return Err("insufficient replica capacity");
        }
        reserved[position] = updated;
    }
    let mut result = nodes.to_vec();
    for (node, &bytes) in result.iter_mut().zip(reserved.iter()) {
        if bytes != 0 {
            node.capacity = node.capacity.reserve(bytes)?;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod multi_replica_capacity_planning_tests {
    use super::*;

    fn node(name: &str, total: u64, used: u64, reserved: u64) -> ReplicaNodeCapacity {
        ReplicaNodeCapacity {
            node: name.into(),
            capacity: Capacity { total, used, reserved },
        }
    }

    #[test]
    fn repeated_node_requests_commit_as_one_atomic_value_plan() {
        let before = [node("a", 10, 2, 0), node("b", 9, 0, 1)];
        let planned = reserve_replica_capacity_plan(
            &before, &[("a", 3), ("b", 4), ("a", 2)]
        ).unwrap();
        assert_eq!(planned[0], node("a", 10, 2, 5));
        assert_eq!(planned[1], node("b", 9, 0, 5));
        assert_eq!(planned[0].capacity.available().unwrap(), 3);
        assert_eq!(planned[1].capacity.available().unwrap(), 4);
        assert_eq!(before[0], node("a", 10, 2, 0));
        assert_eq!(before[1], node("b", 9, 0, 1));
        assert_eq!(
            planned,
            reserve_replica_capacity_plan(
                &before, &[("a", 2), ("a", 3), ("b", 4)]
            ).unwrap()
        );
    }

    #[test]
    fn rejects_partial_capacity_and_invalid_members_without_mutation() {
        let before = [node("a", 10, 2, 0), node("b", 9, 0, 1)];
        assert!(reserve_replica_capacity_plan(
            &before, &[("a", 3), ("b", 9)]
        ).is_err());
        assert!(reserve_replica_capacity_plan(
            &before, &[("a", 3), ("unknown", 1)]
        ).is_err());
        assert_eq!(before[0].capacity.reserved, 0);
        assert!(reserve_replica_capacity_plan(
            &[before[0].clone(), before[0].clone()], &[]
        ).is_err());
        assert!(reserve_replica_capacity_plan(
            &[node("a", 10, 9, 2)], &[]
        ).is_err());
        assert!(reserve_replica_capacity_plan(
            &before, &vec![("a", 0); 65_537]
        ).is_err());
        assert!(reserve_replica_capacity_plan(&[], &[]).is_err());
    }

    #[test]
    fn detects_full_width_reservation_overflow_and_is_order_stable() {
        let before = [node("node_1", u64::MAX, 0, 0)];
        assert!(reserve_replica_capacity_plan(
            &before, &[("node_1", u64::MAX), ("node_1", 1)]
        ).is_err());
        let planned = reserve_replica_capacity_plan(
            &before, &[("node_1", u64::MAX)]
        ).unwrap();
        assert_eq!(planned[0].capacity.available().unwrap(), 0);
    }
}

#[cfg(test)]
mod release_invalid_state_tests {
    use super::*;

    #[test]
    fn rejects_invalid_resulting_states_but_allows_recovery() {
        let overcommitted = Capacity {
            total: 1,
            used: 2,
            reserved: 1,
        };
        assert!(overcommitted.release(1).is_err());
        assert!(overcommitted.release_many(&[1]).is_err());

        let overflowed = Capacity {
            total: u64::MAX,
            used: u64::MAX,
            reserved: 1,
        };
        assert!(overflowed.release(0).is_err());
        assert!(overflowed.release_many(&[]).is_err());

        // The existing API allows a release to heal an overcommit.
        let recoverable = Capacity {
            total: 10,
            used: 9,
            reserved: 2,
        };
        assert_eq!(recoverable.release(1).unwrap().available().unwrap(), 0);

        let valid = Capacity {
            total: 10,
            used: 4,
            reserved: 3,
        };
        assert_eq!(valid.release(2).unwrap().available().unwrap(), 5);
    }
}

#[cfg(test)]
mod batched_replica_reservation_tests {
    use super::*;

    fn reference(
        nodes: &[ReplicaNodeCapacity],
        requests: &[(&str, u64)],
    ) -> Result<Vec<ReplicaNodeCapacity>, &'static str> {
        let mut planned = nodes.to_vec();
        for &(node, bytes) in requests {
            let position = planned.iter().position(|item| item.node == node)
                .ok_or("unknown replica reservation node")?;
            planned[position].capacity = planned[position].capacity.reserve(bytes)?;
        }
        Ok(planned)
    }

    #[test]
    fn batched_totals_match_sequential_plans_for_all_replica_counts() {
        for size in [1usize, 2, 8, 9, 16, 64] {
            let nodes = (0..size).map(|index| ReplicaNodeCapacity {
                node: format!("replica_{index:02}"),
                capacity: Capacity {
                    total: 100_000, used: (index as u64) * 3, reserved: 7,
                },
            }).collect::<Vec<_>>();
            for request_count in [0usize, 1, 8, 9, 128, 4_096, 65_536] {
                let requests = (0..request_count).map(|index| {
                    (nodes[(index * 37 + 11) % size].node.as_str(), (index % 3) as u64)
                }).collect::<Vec<_>>();
                assert_eq!(
                    reserve_replica_capacity_plan(&nodes, &requests),
                    reference(&nodes, &requests),
                    "size={size}, requests={request_count}",
                );
            }
            let mut over_budget = vec![(nodes[0].node.as_str(), 100_000)];
            over_budget.push((nodes[0].node.as_str(), 1));
            assert_eq!(
                reserve_replica_capacity_plan(&nodes, &over_budget).is_err(),
                reference(&nodes, &over_budget).is_err(),
            );
            let mut bad = vec![(nodes[0].node.as_str(), 0); 128];
            bad.push(("foreign", 1));
            assert!(reserve_replica_capacity_plan(&nodes, &bad).is_err());
        }
    }

    #[test]
    fn max_request_batch_preserves_exact_capacity_and_input() {
        let before = [ReplicaNodeCapacity {
            node: "replica".into(),
            capacity: Capacity { total: 65_536, used: 0, reserved: 0 },
        }];
        let requests = vec![("replica", 1); 65_536];
        let result = reserve_replica_capacity_plan(&before, &requests).unwrap();
        assert_eq!(result[0].capacity.reserved, 65_536);
        assert_eq!(result[0].capacity.available().unwrap(), 0);
        assert_eq!(before[0].capacity.reserved, 0);
        assert!(reserve_replica_capacity_plan(
            &before, &vec![("replica", u64::MAX); 65_536]
        ).is_err());
    }
}
