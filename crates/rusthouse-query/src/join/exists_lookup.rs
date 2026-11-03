//! Bounded lookup for equality-correlated EXISTS with an optional inequality.
use crate::ExecutionError;
use std::collections::HashMap;

const MAX_KEYS: usize = 100_000;

/// Integer keys must use matching signedness on both sides of a correlation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoinExistsKey {
    Signed(i128),
    Unsigned(u128),
}

#[derive(Debug, Clone, Copy)]
enum Candidates {
    Any,
    One(JoinExistsKey),
    Multiple,
}

/// A bounded lookup for rows that already passed their inner predicates.
/// A missing key represents SQL NULL. Callers must validate matching types.
pub struct JoinExistsLookup {
    inequality: bool,
    rows: HashMap<JoinExistsKey, Candidates>,
}

impl JoinExistsLookup {
    pub fn new(inequality: bool) -> Self {
        Self {
            inequality,
            rows: HashMap::new(),
        }
    }

    /// Add one inner row; NULL correlation values cannot satisfy equality or inequality.
    pub fn insert(
        &mut self,
        key: Option<JoinExistsKey>,
        unequal: Option<JoinExistsKey>,
    ) -> Result<(), ExecutionError> {
        let Some(key) = key else {
            return Ok(());
        };
        let value = if self.inequality {
            let Some(value) = unequal else {
                return Ok(());
            };
            Candidates::One(value)
        } else {
            Candidates::Any
        };
        if let Some(existing) = self.rows.get_mut(&key) {
            if let (Candidates::One(first), Candidates::One(next)) = (*existing, value) {
                if first != next {
                    *existing = Candidates::Multiple;
                }
            }
        } else {
            if self.rows.len() >= MAX_KEYS {
                return Err(ExecutionError::Invalid(
                    "JOIN EXISTS supports at most 100000 keys".into(),
                ));
            }
            self.rows.insert(key, value);
        }
        Ok(())
    }

    /// Return whether an inner row matches; NOT EXISTS is the inverse.
    pub fn exists(&self, key: Option<JoinExistsKey>, unequal: Option<JoinExistsKey>) -> bool {
        let Some(key) = key else {
            return false;
        };
        let Some(candidates) = self.rows.get(&key) else {
            return false;
        };
        match candidates {
            Candidates::Any => true,
            Candidates::One(first) => unequal.is_some_and(|value| value != *first),
            Candidates::Multiple => unequal.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(value: u128) -> Option<JoinExistsKey> {
        Some(JoinExistsKey::Unsigned(value))
    }

    #[test]
    fn inequality_matches_naive_exists_for_duplicates_and_nulls() {
        let values = [None, key(0), key(1), key(2)];
        for eq_a in values {
            for neq_a in values {
                for eq_b in values {
                    for neq_b in values {
                        let rows = [(eq_a, neq_a), (eq_b, neq_b)];
                        let mut lookup = JoinExistsLookup::new(true);
                        for (eq, neq) in rows {
                            lookup.insert(eq, neq).unwrap();
                        }
                        for outer_eq in values {
                            for outer_neq in values {
                                let expected = rows.iter().any(|(eq, neq)| {
                                    eq.is_some()
                                        && neq.is_some()
                                        && outer_eq.is_some()
                                        && outer_neq.is_some()
                                        && eq == &outer_eq
                                        && neq != &outer_neq
                                });
                                assert_eq!(
                                    lookup.exists(outer_eq, outer_neq),
                                    expected,
                                    "{rows:?} {outer_eq:?} {outer_neq:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn equality_only_ignores_inequality_values_and_skips_null_keys() {
        let mut lookup = JoinExistsLookup::new(false);
        lookup.insert(None, key(1)).unwrap();
        lookup.insert(key(1), None).unwrap();
        assert!(lookup.exists(key(1), None));
        assert!(!lookup.exists(None, key(1)));
        assert!(!lookup.exists(key(2), None));
    }

    #[test]
    fn signed_full_width_values_keep_their_identity() {
        let mut lookup = JoinExistsLookup::new(true);
        let min = Some(JoinExistsKey::Signed(i128::MIN));
        let max = Some(JoinExistsKey::Signed(i128::MAX));
        lookup.insert(min, max).unwrap();
        assert!(!lookup.exists(min, max));
        assert!(lookup.exists(min, min));
        assert!(!lookup.exists(max, min));
    }

    #[test]
    fn bound_counts_unique_keys_not_duplicate_rows() {
        let mut lookup = JoinExistsLookup::new(true);
        for value in 0..MAX_KEYS {
            lookup.insert(key(value as u128), key(1)).unwrap();
        }
        assert!(lookup.insert(key(0), key(2)).is_ok());
        assert!(lookup.exists(key(0), key(1)));
        assert!(lookup.insert(key(MAX_KEYS as u128), key(1)).is_err());
        assert!(!lookup.exists(key(MAX_KEYS as u128), key(2)));
    }
}
