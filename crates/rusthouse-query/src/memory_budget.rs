//! Logical memory accounting for operators that explicitly reserve buffers.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct QueryMemoryBudget {
    used: Arc<AtomicUsize>,
    max_bytes: usize,
}

#[derive(Debug)]
pub struct MemoryReservation {
    budget: QueryMemoryBudget,
    bytes: usize,
}

impl QueryMemoryBudget {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            used: Arc::new(AtomicUsize::new(0)),
            max_bytes,
        }
    }

    pub fn used_bytes(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }

    pub fn reserve(&self, bytes: usize) -> Result<MemoryReservation, &'static str> {
        let mut current = self.used.load(Ordering::Acquire);
        loop {
            let next = current
                .checked_add(bytes)
                .filter(|&next| next <= self.max_bytes)
                .ok_or("query memory budget exceeded")?;
            match self.used.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(MemoryReservation { budget: self.clone(), bytes }),
                Err(observed) => current = observed,
            }
        }
    }
}

impl Drop for MemoryReservation {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservations_share_a_budget_and_release_on_drop() {
        let budget = QueryMemoryBudget::new(10);
        let other = budget.clone();
        let first = budget.reserve(7).unwrap();
        assert!(other.reserve(4).is_err());
        let second = other.reserve(3).unwrap();
        assert_eq!(budget.used_bytes(), 10);
        drop(first);
        assert_eq!(budget.used_bytes(), 3);
        drop(second);
        assert_eq!(budget.used_bytes(), 0);
    }

    #[test]
    fn arithmetic_overflow_cannot_bypass_the_limit() {
        let budget = QueryMemoryBudget::new(usize::MAX);
        let _first = budget.reserve(usize::MAX).unwrap();
        assert!(budget.reserve(1).is_err());
    }
}
