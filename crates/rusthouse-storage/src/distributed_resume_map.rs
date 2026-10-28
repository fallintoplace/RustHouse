//! Bounded completion map for retrying interrupted chunk transfers.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeMap {
    completed: Vec<bool>,
}

impl ResumeMap {
    pub fn new(chunks: usize) -> Result<Self, &'static str> {
        if chunks > 65_536 {
            return Err("resume map exceeds chunk limit");
        }
        Ok(Self {
            completed: vec![false; chunks],
        })
    }

    pub fn mark_complete(&mut self, index: usize) -> Result<(), &'static str> {
        let value = self
            .completed
            .get_mut(index)
            .ok_or("chunk index out of bounds")?;
        *value = true;
        Ok(())
    }

    pub fn remaining(&self) -> Vec<usize> {
        self.completed
            .iter()
            .enumerate()
            .filter_map(|(index, complete)| (!complete).then_some(index))
            .collect()
    }

    pub fn complete(&self) -> bool {
        self.completed.iter().all(|value| *value)
    }

    /// Stable ASCII encoding for persisting transfer progress.
    pub fn encode(&self) -> String {
        self.completed
            .iter()
            .map(|value| if *value { '1' } else { '0' })
            .collect()
    }

    pub fn decode(text: &str) -> Result<Self, &'static str> {
        let mut result = Self::new(text.len())?;
        for (index, byte) in text.bytes().enumerate() {
            match byte {
                b'0' => {}
                b'1' => result.mark_complete(index)?,
                _ => return Err("invalid resume map byte"),
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoints_and_resumes_missing_chunks() {
        let mut map = ResumeMap::new(4).unwrap();
        map.mark_complete(0).unwrap();
        map.mark_complete(2).unwrap();
        let recovered = ResumeMap::decode(&map.encode()).unwrap();
        assert_eq!(recovered.remaining(), vec![1, 3]);
        assert!(!recovered.complete());
    }

    #[test]
    fn rejects_invalid_progress_and_bounds() {
        assert!(ResumeMap::new(65_537).is_err());
        assert!(ResumeMap::decode("10x01").is_err());
        assert!(ResumeMap::new(1).unwrap().mark_complete(2).is_err());
    }
}