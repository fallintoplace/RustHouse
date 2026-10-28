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
        if bytes > self.reserved {
            return Err("cannot commit unreserved bytes");
        }
        Ok(Self {
            used: self
                .used
                .checked_add(bytes)
                .ok_or("replica usage overflow")?,
            reserved: self.reserved - bytes,
            ..self
        })
    }

    pub fn release(self, bytes: u64) -> Result<Self, &'static str> {
        if bytes > self.reserved {
            return Err("cannot release unreserved bytes");
        }
        Ok(Self {
            reserved: self.reserved - bytes,
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
}
