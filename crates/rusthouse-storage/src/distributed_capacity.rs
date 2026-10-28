//! Checked storage-capacity accounting for planned object transfers.

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

    /// Reserve bytes without changing the used-byte count.
    pub fn reserve(self, bytes: u64) -> Result<Self, &'static str> {
        if bytes > self.available()? {
            return Err("insufficient replica capacity");
        }
        Ok(Self {
            reserved: self
                .reserved
                .checked_add(bytes)
                .ok_or("replica reservation overflow")?,
            ..self
        })
    }

    /// Convert reserved bytes into committed usage.
    pub fn commit(self, bytes: u64) -> Result<Self, &'static str> {
        let reserved = self
            .reserved
            .checked_sub(bytes)
            .ok_or("cannot commit unreserved bytes")?;
        self.available()?;
        Ok(Self {
            used: self
                .used
                .checked_add(bytes)
                .ok_or("replica usage overflow")?,
            reserved,
            ..self
        })
    }

    pub fn release(self, bytes: u64) -> Result<Self, &'static str> {
        Ok(Self {
            reserved: self
                .reserved
                .checked_sub(bytes)
                .ok_or("cannot release unreserved bytes")?,
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
        if object_sizes.len() > 65_536 {
            return Err("replica object batch exceeds limit");
        }
        let bytes = object_sizes.iter().try_fold(0u64, |total, &size| {
            total.checked_add(size).ok_or("replica batch size overflow")
        })?;
        self.reserve(bytes)
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
    fn refuses_overflow_and_unbounded_batches() {
        let capacity = Capacity { total: u64::MAX, used: 0, reserved: 0 };
        assert!(capacity.reserve_many(&[u64::MAX, 1]).is_err());
        assert!(capacity.reserve_many(&vec![0; 65_537]).is_err());
        assert_eq!(capacity.reserve_many(&[]).unwrap(), capacity);
    }
}
