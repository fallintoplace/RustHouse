//! Shared cancellation and deadline state for future query operators.
use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct ExecutionControl {
    cancelled: Arc<AtomicBool>,
    deadline: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionStop {
    Cancelled,
    DeadlineExceeded,
}

impl fmt::Display for ExecutionStop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("query cancelled"),
            Self::DeadlineExceeded => f.write_str("query deadline exceeded"),
        }
    }
}

impl Error for ExecutionStop {}

impl Default for ExecutionControl {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionControl {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline: None,
        }
    }

    pub fn with_deadline(deadline: Instant) -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline: Some(deadline),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn check(&self) -> Result<(), ExecutionStop> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(ExecutionStop::Cancelled);
        }
        if self.deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(ExecutionStop::DeadlineExceeded);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_reaches_cloned_workers() {
        let worker = ExecutionControl::new();
        let caller = worker.clone();
        assert_eq!(worker.check(), Ok(()));
        caller.cancel();
        assert_eq!(worker.check(), Err(ExecutionStop::Cancelled));
    }

    #[test]
    fn expired_deadline_is_detected() {
        let control = ExecutionControl::with_deadline(Instant::now());
        assert_eq!(control.check(), Err(ExecutionStop::DeadlineExceeded));
    }
}
