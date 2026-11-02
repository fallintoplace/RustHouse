//! Bounded adaptive heartbeat suspicion using an exponential arrival model.
//! Suspicion is diagnostic only, never permission for a replica election.

use std::collections::VecDeque;

const MAX_WINDOW: usize = 256;
const LN_10: f64 = std::f64::consts::LN_10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatDetector {
    last_ms: Option<u64>,
    intervals_ms: VecDeque<u64>,
    sum_intervals: u128,
    window: usize,
    minimum_samples: usize,
}

impl HeartbeatDetector {
    /// A larger history smooths jitter while retaining a bounded memory cost.
    pub fn new(window: usize, minimum_samples: usize) -> Result<Self, &'static str> {
        if window == 0
            || window > MAX_WINDOW
            || minimum_samples == 0
            || minimum_samples > window
        {
            return Err("invalid heartbeat detector window");
        }
        Ok(Self {
            last_ms: None,
            intervals_ms: VecDeque::with_capacity(window),
            sum_intervals: 0,
            window,
            minimum_samples,
        })
    }

    /// Record a heartbeat with strictly increasing caller-supplied timestamps.
    /// Invalid input does not change history or the last accepted timestamp.
    pub fn observe(&mut self, now_ms: u64) -> Result<(), &'static str> {
        let interval = match self.last_ms {
            Some(previous) => {
                if now_ms <= previous {
                    return Err("heartbeat timestamp did not advance");
                }
                Some(now_ms - previous)
            }
            None => None,
        };
        if let Some(interval) = interval {
            if self.intervals_ms.len() == self.window {
                let removed = self
                    .intervals_ms
                    .pop_front()
                    .expect("full detector history is nonempty");
                self.sum_intervals -= u128::from(removed);
            }
            self.intervals_ms.push_back(interval);
            self.sum_intervals += u128::from(interval);
        }
        self.last_ms = Some(now_ms);
        Ok(())
    }

    pub fn samples(&self) -> usize {
        self.intervals_ms.len()
    }

    /// Compute phi = -log10(P(next heartbeat arrives later than elapsed)).
    ///
    /// For a memoryless exponential interarrival model this simplifies to
    /// elapsed / (sample_mean * ln(10)). Return None until the warmup window
    /// has enough intervals. This is not a Gaussian phi-accrual detector.
    pub fn phi(&self, now_ms: u64) -> Result<Option<f64>, &'static str> {
        let Some(last) = self.last_ms else {
            return Ok(None);
        };
        if now_ms < last {
            return Err("heartbeat suspicion clock moved backwards");
        }
        if self.intervals_ms.len() < self.minimum_samples {
            return Ok(None);
        }
        let mean_ms = self.sum_intervals as f64 / self.intervals_ms.len() as f64;
        let elapsed = (now_ms - last) as f64;
        Ok(Some(elapsed / (mean_ms * LN_10)))
    }

    /// Evaluate a finite, positive caller threshold without deciding elections.
    pub fn suspicious(
        &self,
        now_ms: u64,
        threshold: f64,
    ) -> Result<Option<bool>, &'static str> {
        if !threshold.is_finite() || threshold <= 0.0 || threshold > 100.0 {
            return Err("invalid heartbeat suspicion threshold");
        }
        Ok(self.phi(now_ms)?.map(|phi| phi >= threshold))
    }

    /// Clear observations after an explicit node restart or epoch change.
    pub fn reset(&mut self) {
        self.last_ms = None;
        self.intervals_ms.clear();
        self.sum_intervals = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_phi_matches_known_fixed_interval_model() {
        let mut detector = HeartbeatDetector::new(8, 4).unwrap();
        for index in 0..=4 {
            detector.observe(index * 100).unwrap();
        }
        assert_eq!(detector.samples(), 4);
        assert!(detector.phi(400).unwrap().unwrap().abs() < 0.000_001);
        let phi = detector.phi(630).unwrap().unwrap();
        assert!((phi - 1.0).abs() < 0.002);
        assert_eq!(detector.suspicious(630, 0.9).unwrap(), Some(true));
        assert_eq!(detector.suspicious(630, 1.1).unwrap(), Some(false));
    }

    #[test]
    fn oldest_samples_are_evicted_without_losing_precision() {
        let mut detector = HeartbeatDetector::new(3, 2).unwrap();
        for time in [0, 10, 30, 60, 100] {
            detector.observe(time).unwrap();
        }
        assert_eq!(detector.samples(), 3);
        let expected = 40.0 / (30.0 * LN_10);
        assert!((detector.phi(140).unwrap().unwrap() - expected).abs() < 1e-12);
    }

    #[test]
    fn rejects_backward_and_duplicate_heartbeats_without_mutation() {
        let mut detector = HeartbeatDetector::new(4, 2).unwrap();
        detector.observe(100).unwrap();
        detector.observe(200).unwrap();
        let before = detector.clone();
        assert!(detector.observe(200).is_err());
        assert!(detector.observe(199).is_err());
        assert!(detector.phi(199).is_err());
        assert_eq!(detector, before);
    }

    #[test]
    fn warmup_threshold_validation_and_reset() {
        let mut detector = HeartbeatDetector::new(4, 3).unwrap();
        assert_eq!(detector.phi(0).unwrap(), None);
        assert_eq!(detector.suspicious(0, 2.0).unwrap(), None);
        assert!(detector.suspicious(0, f64::NAN).is_err());
        assert!(detector.suspicious(0, f64::INFINITY).is_err());
        assert!(detector.suspicious(0, 0.0).is_err());
        for time in [1, 2, 3] {
            detector.observe(time).unwrap();
        }
        assert_eq!(detector.phi(3).unwrap(), None);
        detector.observe(4).unwrap();
        assert_eq!(detector.phi(4).unwrap(), Some(0.0));
        detector.reset();
        assert_eq!(detector.samples(), 0);
        assert_eq!(detector.phi(4).unwrap(), None);
        assert!(HeartbeatDetector::new(0, 1).is_err());
        assert!(HeartbeatDetector::new(3, 4).is_err());
    }

    #[test]
    fn large_timestamps_do_not_overflow_the_sample_sum() {
        let mut detector = HeartbeatDetector::new(2, 2).unwrap();
        for time in [0, u64::MAX / 2, u64::MAX - 1] {
            detector.observe(time).unwrap();
        }
        assert!(detector.phi(u64::MAX).unwrap().unwrap().is_finite());
    }
}


impl HeartbeatDetector {
    /// Return a robust empirical heartbeat deadline without altering history.
    ///
    /// The nearest-rank quantile is calculated from bounded observed intervals.
    /// Unlike the exponential phi estimate, it does not assume memoryless
    /// arrivals. This is diagnostic; it must never grant replica ownership.
    pub fn quantile_deadline_ms(
        &self,
        percentile: f64,
        tolerance: u32,
    ) -> Result<Option<u64>, &'static str> {
        if !percentile.is_finite()
            || percentile <= 0.0
            || percentile > 1.0
            || tolerance == 0
            || tolerance > 16
        {
            return Err("invalid empirical heartbeat timeout settings");
        }
        let Some(last) = self.last_ms else {
            return Ok(None);
        };
        if self.intervals_ms.len() < self.minimum_samples {
            return Ok(None);
        }
        let mut intervals = self.intervals_ms.iter().copied().collect::<Vec<_>>();
        intervals.sort_unstable();
        let rank = ((intervals.len() as f64 * percentile).ceil() as usize)
            .saturating_sub(1)
            .min(intervals.len() - 1);
        let delay = intervals[rank]
            .checked_mul(u64::from(tolerance))
            .ok_or("empirical heartbeat deadline overflow")?;
        let deadline = last
            .checked_add(delay)
            .ok_or("empirical heartbeat deadline overflow")?;
        Ok(Some(deadline))
    }

    /// Evaluate whether the chosen quantile-based deadline has expired.
    /// A timestamp preceding the last observed heartbeat is invalid.
    pub fn missed_quantile_deadline(
        &self,
        now_ms: u64,
        percentile: f64,
        tolerance: u32,
    ) -> Result<Option<bool>, &'static str> {
        if self.last_ms.is_some_and(|last| now_ms < last) {
            return Err("heartbeat suspicion clock moved backwards");
        }
        Ok(self
            .quantile_deadline_ms(percentile, tolerance)?
            .map(|deadline| now_ms > deadline))
    }
}

#[cfg(test)]
mod empirical_deadline_tests {
    use super::*;

    #[test]
    fn quantiles_react_to_tail_latency_without_changing_the_history() {
        let mut detector = HeartbeatDetector::new(8, 4).unwrap();
        for time in [0, 100, 200, 300, 1300] {
            detector.observe(time).unwrap();
        }
        let prior = detector.clone();
        assert_eq!(detector.quantile_deadline_ms(0.75, 3).unwrap(), Some(1600));
        assert_eq!(detector.quantile_deadline_ms(1.0, 2).unwrap(), Some(3300));
        assert_eq!(
            detector.missed_quantile_deadline(1600, 0.75, 3).unwrap(),
            Some(false)
        );
        assert_eq!(
            detector.missed_quantile_deadline(1601, 0.75, 3).unwrap(),
            Some(true)
        );
        assert_eq!(detector, prior);
    }

    #[test]
    fn invalid_percentiles_and_timestamp_overflow_fail_closed() {
        let mut detector = HeartbeatDetector::new(2, 1).unwrap();
        assert_eq!(detector.quantile_deadline_ms(0.5, 2).unwrap(), None);
        for percentile in [0.0, -1.0, 1.01, f64::NAN, f64::INFINITY] {
            assert!(detector.quantile_deadline_ms(percentile, 1).is_err());
        }
        assert!(detector.quantile_deadline_ms(0.5, 0).is_err());
        assert!(detector.quantile_deadline_ms(0.5, 17).is_err());
        detector.observe(u64::MAX - 10).unwrap();
        detector.observe(u64::MAX - 1).unwrap();
        assert!(detector.quantile_deadline_ms(0.5, 2).is_err());
        assert!(detector.missed_quantile_deadline(0, 0.5, 1).is_err());
    }
}
