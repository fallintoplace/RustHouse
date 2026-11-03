//! Bounded in-flight byte credits for replica transfers.
//! ACK checksums must come from a separate independently verified receiver.
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferFrame {
    pub sequence: u64,
    pub bytes: u64,
    pub checksum: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferCreditWindow {
    budget: u64,
    in_flight: u64,
    next_sequence: u64,
    frames: VecDeque<TransferFrame>,
}

impl TransferCreditWindow {
    pub fn new(budget: u64) -> Result<Self, &'static str> {
        if budget == 0 { return Err("transfer byte credit budget must be nonzero"); }
        Ok(Self { budget, in_flight: 0, next_sequence: 1, frames: VecDeque::new() })
    }


    /// Adjust backpressure without dropping in-flight frames or sequence state.
    pub fn set_budget(&mut self, budget: u64) -> Result<(), &'static str> {
        if budget == 0 || budget < self.in_flight {
            return Err("transfer byte budget is below in-flight bytes");
        }
        self.budget = budget;
        Ok(())
    }

    pub fn in_flight_bytes(&self) -> u64 { self.in_flight }
    pub fn available_bytes(&self) -> u64 { self.budget - self.in_flight }

    pub fn reserve(&mut self, bytes: u64, checksum: u32)
        -> Result<TransferFrame, &'static str>
    {
        if bytes == 0 || bytes > self.available_bytes() || self.frames.len() >= 4096 {
            return Err("transfer window exhausted or frame invalid");
        }
        let next = self.next_sequence.checked_add(1).ok_or("transfer sequence overflow")?;
        let frame = TransferFrame {
            sequence: self.next_sequence, bytes, checksum,
        };
        self.frames.push_back(frame);
        self.next_sequence = next;
        self.in_flight += bytes;
        Ok(frame)
    }

    /// Release credits only after validating the whole contiguous ACK prefix.
    /// A mismatched ACK does not mutate the flight window.
    pub fn acknowledge_verified(
        &mut self,
        acknowledgements: &[(u64, u32)],
    ) -> Result<u64, &'static str> {
        if acknowledgements.len() > self.frames.len() {
            return Err("too many verified transfer acknowledgements");
        }
        let mut bytes = 0u64;
        for ((sequence, checksum), expected) in acknowledgements.iter()
            .zip(self.frames.iter())
        {
            if *sequence != expected.sequence || *checksum != expected.checksum {
                return Err("missing, reordered, or conflicting transfer acknowledgement");
            }
            bytes += expected.bytes;
        }
        for _ in acknowledgements {
            self.frames.pop_front();
        }
        self.in_flight -= bytes;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credits_are_only_returned_for_verified_prefix() {
        let mut window = TransferCreditWindow::new(10).unwrap();
        let first = window.reserve(6, 33).unwrap();
        let second = window.reserve(4, 55).unwrap();
        assert_eq!(window.available_bytes(), 0);
        assert!(window.reserve(1, 1).is_err());
        assert!(window.acknowledge_verified(&[(second.sequence, 55)]).is_err());
        assert_eq!(window.available_bytes(), 0);
        assert_eq!(window.acknowledge_verified(&[(first.sequence, 33)]).unwrap(), 6);
        assert_eq!(window.available_bytes(), 6);
        assert_eq!(window.acknowledge_verified(&[(second.sequence, 55)]).unwrap(), 4);
        assert_eq!(window.in_flight_bytes(), 0);
    }

    #[test]
    fn false_ack_is_atomic() {
        let mut window = TransferCreditWindow::new(10).unwrap();
        let a = window.reserve(4, 11).unwrap();
        let b = window.reserve(4, 12).unwrap();
        let before = window.clone();
        assert!(window.acknowledge_verified(&[(a.sequence, 11), (b.sequence, 77)]).is_err());
        assert_eq!(window, before);
    }
}

#[cfg(test)]
mod transfer_budget_tests {
    use super::*;

    #[test]
    fn resizing_never_overcommits_existing_frames() {
        let mut window = TransferCreditWindow::new(10).unwrap();
        let first = window.reserve(6, 77).unwrap();
        let before = window.clone();
        assert!(window.set_budget(5).is_err());
        assert!(window.set_budget(0).is_err());
        assert_eq!(window, before);

        window.set_budget(6).unwrap();
        assert_eq!(window.available_bytes(), 0);
        assert_eq!(window.acknowledge_verified(&[(first.sequence, 77)]).unwrap(), 6);
        window.set_budget(3).unwrap();
        assert_eq!(window.available_bytes(), 3);
        let next = window.reserve(3, 88).unwrap();
        assert_eq!(next.sequence, first.sequence + 1);
        assert_eq!(window.in_flight_bytes(), 3);
    }

    #[test]
    fn increasing_budget_keeps_outstanding_receipts() {
        let mut window = TransferCreditWindow::new(2).unwrap();
        let frame = window.reserve(2, 5).unwrap();
        window.set_budget(10).unwrap();
        assert_eq!(window.available_bytes(), 8);
        assert_eq!(window.acknowledge_verified(&[(frame.sequence, 5)]).unwrap(), 2);
        assert_eq!(window.available_bytes(), 10);
    }
}
