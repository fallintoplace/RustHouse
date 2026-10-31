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
        Ok(Self {
            reserved: self.remaining_reservation(bytes, "cannot release unreserved bytes")?,
            ..self
        })
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
