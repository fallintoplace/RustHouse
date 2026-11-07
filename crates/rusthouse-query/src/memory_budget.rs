//! Logical memory accounting for operators that explicitly reserve buffers.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// Atomically charge a bounded byte counter without overflowing.
/// Callers own the matching release after a successful reservation.
pub(crate) fn try_reserve_atomic(used: &AtomicUsize, bytes: usize, max_bytes: usize) -> bool {
    let mut current = used.load(Ordering::Acquire);
    loop {
        let Some(next) = current.checked_add(bytes).filter(|&next| next <= max_bytes) else {
            return false;
        };
        match used.compare_exchange_weak(
            current,
            next,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(observed) => current = observed,
        }
    }
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
        if !try_reserve_atomic(&self.used, bytes, self.max_bytes) {
            return Err("query memory budget exceeded");
        }
        Ok(MemoryReservation {
            budget: self.clone(),
            bytes,
        })
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
    fn low_level_atomic_reservation_respects_capacity_and_overflow() {
        let used = AtomicUsize::new(0);
        assert!(try_reserve_atomic(&used, 7, 10));
        assert!(!try_reserve_atomic(&used, 4, 10));
        assert_eq!(used.load(Ordering::Acquire), 7);
        assert!(try_reserve_atomic(&used, 3, 10));
        assert!(!try_reserve_atomic(&used, 1, 10));
        assert_eq!(used.load(Ordering::Acquire), 10);

        let huge = AtomicUsize::new(usize::MAX);
        assert!(!try_reserve_atomic(&huge, 1, usize::MAX));
    }

    #[test]
    fn arithmetic_overflow_cannot_bypass_the_limit() {
        let budget = QueryMemoryBudget::new(usize::MAX);
        let _first = budget.reserve(usize::MAX).unwrap();
        assert!(budget.reserve(1).is_err());
    }
}
