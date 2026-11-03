//! Observation-only counters for future physical EXPLAIN ANALYZE output.
use std::time::{Duration, Instant};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OperatorMetrics {
    pub scanned_rows: u64,
    pub emitted_rows: u64,
    pub decoded_bytes: u64,
    pub elapsed: Duration,
}

#[derive(Debug)]
pub struct OperatorMeter {
    started: Instant,
    scanned_rows: u64,
    emitted_rows: u64,
    decoded_bytes: u64,
}

impl Default for OperatorMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl OperatorMeter {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            scanned_rows: 0,
            emitted_rows: 0,
            decoded_bytes: 0,
        }
    }

    pub fn record_scan(&mut self, rows: u64, decoded_bytes: u64) {
        self.scanned_rows = self.scanned_rows.saturating_add(rows);
        self.decoded_bytes = self.decoded_bytes.saturating_add(decoded_bytes);
    }

    pub fn record_output(&mut self, rows: u64) {
        self.emitted_rows = self.emitted_rows.saturating_add(rows);
    }

    pub fn finish(self) -> OperatorMetrics {
        OperatorMetrics {
            scanned_rows: self.scanned_rows,
            emitted_rows: self.emitted_rows,
            decoded_bytes: self.decoded_bytes,
            elapsed: self.started.elapsed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_metrics_accumulate_without_integer_wraparound() {
        let mut meter = OperatorMeter::new();
        meter.record_scan(u64::MAX, 5);
        meter.record_scan(10, u64::MAX);
        meter.record_output(3);
        let metrics = meter.finish();
        assert_eq!(metrics.scanned_rows, u64::MAX);
        assert_eq!(metrics.decoded_bytes, u64::MAX);
        assert_eq!(metrics.emitted_rows, 3);
    }
}
