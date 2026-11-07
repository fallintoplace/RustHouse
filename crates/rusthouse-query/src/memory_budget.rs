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

/// Result of charging retained operator buffers, before updating counters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RetainedBudgetError {
    Overflow,
    LimitExceeded,
}

/// Check a retained byte charge without modifying the counter on failure.
/// Operators map these errors to their existing, operator-specific messages.
pub(crate) fn checked_retained_bytes(
    used: usize,
    added: usize,
    limit: usize,
) -> Result<usize, RetainedBudgetError> {
    let next = used.checked_add(added).ok_or(RetainedBudgetError::Overflow)?;
    if next > limit {
        return Err(RetainedBudgetError::LimitExceeded);
    }
    Ok(next)
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
                Ok(_) => {
                    return Ok(MemoryReservation {
                        budget: self.clone(),
                        bytes,
                    });
                }
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
    fn checked_retained_charges_keep_failed_counters_unchanged() {
        assert_eq!(checked_retained_bytes(7, 3, 10), Ok(10));
        assert_eq!(
            checked_retained_bytes(7, 4, 10),
            Err(RetainedBudgetError::LimitExceeded)
        );
        assert_eq!(
            checked_retained_bytes(usize::MAX, 1, usize::MAX),
            Err(RetainedBudgetError::Overflow)
        );
        assert_eq!(checked_retained_bytes(0, 0, 0), Ok(0));
    }

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
