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

    /// Estimate remaining milliseconds before the supplied phi threshold.
    ///
    /// This returns a floating-point delay so a scheduler can round upward
    /// instead of firing prematurely. It is diagnostic, not an election timer.
    /// Return None until sufficient heartbeat intervals have been observed.
    pub fn remaining_until_suspicion_ms(
        &self,
        now_ms: u64,
        threshold: f64,
    ) -> Result<Option<f64>, &'static str> {
        if !threshold.is_finite() || threshold <= 0.0 || threshold > 100.0 {
            return Err("invalid heartbeat suspicion threshold");
        }
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
        let target_elapsed = threshold * mean_ms * LN_10;
        let elapsed = (now_ms - last) as f64;
        Ok(Some((target_elapsed - elapsed).max(0.0)))
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
    fn predicted_suspicion_delay_tracks_phi_and_warmup() {
        let mut detector = HeartbeatDetector::new(8, 4).unwrap();
        assert_eq!(detector.remaining_until_suspicion_ms(0, 1.0).unwrap(), None);
        for index in 0..=4 {
            detector.observe(index * 100).unwrap();
        }
        let remaining = detector.remaining_until_suspicion_ms(400, 1.0).unwrap().unwrap();
        assert!((remaining - 100.0 * LN_10).abs() < 1e-9);
        assert!(detector.remaining_until_suspicion_ms(630, 1.0).unwrap().unwrap() < 1.0);
        assert_eq!(detector.remaining_until_suspicion_ms(631, 1.0).unwrap(), Some(0.0));
        assert!(detector.remaining_until_suspicion_ms(399, 1.0).is_err());
        assert!(detector.remaining_until_suspicion_ms(631, f64::NAN).is_err());
        assert!(detector.remaining_until_suspicion_ms(631, 0.0).is_err());
        detector.reset();
        assert_eq!(detector.remaining_until_suspicion_ms(700, 1.0).unwrap(), None);
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
