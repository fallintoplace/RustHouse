//! Quorum policy validation for future replicated storage writes.
//! This module does not implement consensus or distributed transactions.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuorumPolicy {
    pub replicas: usize,
    pub reads: usize,
    pub writes: usize,
}

impl QuorumPolicy {
    /// Require overlapping read/write and write/write quorums.
    pub fn new(replicas: usize, reads: usize, writes: usize) -> Result<Self, &'static str> {
        if replicas == 0
            || replicas > 16
            || reads == 0
            || writes == 0
            || reads > replicas
            || writes > replicas
            || reads + writes <= replicas
            || writes <= replicas / 2
        {
            return Err("unsafe replica quorum policy");
        }
        Ok(Self {
            replicas,
            reads,
            writes,
        })
    }

    pub fn can_read(self, unique_acks: usize) -> bool {
        unique_acks >= self.reads && unique_acks <= self.replicas
    }

    pub fn can_write(self, unique_acks: usize) -> bool {
        unique_acks >= self.writes && unique_acks <= self.replicas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_intersecting_quorums() {
        let policy = QuorumPolicy::new(3, 2, 2).unwrap();
        assert!(policy.can_write(2));
        assert!(!policy.can_read(1));
        assert!(!policy.can_write(4));
        assert_eq!(QuorumPolicy::new(5, 2, 4).unwrap().writes, 4);
    }

    #[test]
    fn refuses_non_intersecting_or_unbounded_policies() {
        assert!(QuorumPolicy::new(3, 1, 2).is_err());
        assert!(QuorumPolicy::new(4, 2, 2).is_err());
        assert!(QuorumPolicy::new(0, 0, 0).is_err());
        assert!(QuorumPolicy::new(17, 9, 9).is_err());
    }
}