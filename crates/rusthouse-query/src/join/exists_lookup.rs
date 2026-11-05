//! Bounded lookup for equality-correlated EXISTS with an optional inequality.
use crate::ExecutionError;
use std::collections::HashMap;

const MAX_KEYS: usize = 200_000;

/// Integer keys must use matching signedness on both sides of a correlation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoinExistsKey {
    Signed(i128),
    Unsigned(u128),
}

#[derive(Debug, Clone, Copy)]
enum Candidates<K> {
    Any,
    One(K),
    Multiple,
}

enum LookupRows {
    General(HashMap<JoinExistsKey, Candidates<JoinExistsKey>>),
    Compact64 {
        signed: bool,
        rows: HashMap<u64, Candidates<u64>>,
    },
}

/// A bounded lookup for rows that already passed their inner predicates.
/// A missing key represents SQL NULL. Callers must validate matching types.
pub struct JoinExistsLookup {
    inequality: bool,
    rows: LookupRows,
}

impl JoinExistsLookup {
    pub fn new(inequality: bool) -> Self {
        Self {
            inequality,
            rows: LookupRows::General(HashMap::new()),
        }
    }

    /// Use only when the bound SQL columns are signed or unsigned integers up to 64 bits.
    pub(crate) fn new_compact64(inequality: bool, signed: bool) -> Self {
        Self {
            inequality,
            rows: LookupRows::Compact64 {
                signed,
                rows: HashMap::new(),
            },
        }
    }

    fn encode64(key: JoinExistsKey, signed: bool) -> Result<u64, ExecutionError> {
        // Signed and unsigned keys never share one bound lookup, so signed i64 bit patterns
        // can use the same compact hash key without losing equality identity.
        match (signed, key) {
            (false, JoinExistsKey::Unsigned(value)) => u64::try_from(value).map_err(|_| {
                ExecutionError::Invalid("JOIN EXISTS key does not fit compact 64-bit lookup".into())
            }),
            (true, JoinExistsKey::Signed(value)) => i64::try_from(value)
                .map(|value| u64::from_ne_bytes(value.to_ne_bytes()))
                .map_err(|_| {
                    ExecutionError::Invalid(
                        "JOIN EXISTS key does not fit compact 64-bit lookup".into(),
                    )
                }),
            _ => Err(ExecutionError::Invalid(
                "JOIN EXISTS key type does not fit compact 64-bit lookup".into(),
            )),
        }
    }

    fn insert_candidate<K: Copy + Eq + std::hash::Hash>(
        rows: &mut HashMap<K, Candidates<K>>,
        key: K,
        value: Candidates<K>,
    ) -> Result<(), ExecutionError> {
        if let Some(existing) = rows.get_mut(&key) {
            if let (Candidates::One(first), Candidates::One(next)) = (*existing, value) {
                if first != next {
                    *existing = Candidates::Multiple;
                }
            }
        } else {
            if rows.len() >= MAX_KEYS {
                return Err(ExecutionError::Invalid(
                    "JOIN EXISTS supports at most 200000 keys".into(),
                ));
            }
            rows.insert(key, value);
        }
        Ok(())
    }

    fn exists_in<K: Copy + Eq + std::hash::Hash>(
        rows: &HashMap<K, Candidates<K>>,
        key: Option<K>,
        unequal: Option<K>,
    ) -> bool {
        let Some(key) = key else {
            return false;
        };
        let Some(candidates) = rows.get(&key) else {
            return false;
        };
        match candidates {
            Candidates::Any => true,
            Candidates::One(first) => unequal.is_some_and(|value| value != *first),
            Candidates::Multiple => unequal.is_some(),
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
        let unequal = if self.inequality {
            let Some(unequal) = unequal else {
                return Ok(());
            };
            Some(unequal)
        } else {
            None
        };
        match &mut self.rows {
            LookupRows::General(rows) => {
                let value = unequal.map_or(Candidates::Any, Candidates::One);
                Self::insert_candidate(rows, key, value)
            }
            LookupRows::Compact64 { signed, rows } => {
                let key = Self::encode64(key, *signed)?;
                let value = match unequal {
                    Some(unequal) => Candidates::One(Self::encode64(unequal, *signed)?),
                    None => Candidates::Any,
                };
                Self::insert_candidate(rows, key, value)
            }
        }
    }

    /// Return whether an inner row matches; NOT EXISTS is the inverse.
    pub fn exists(&self, key: Option<JoinExistsKey>, unequal: Option<JoinExistsKey>) -> bool {
        match &self.rows {
            LookupRows::General(rows) => Self::exists_in(rows, key, unequal),
            LookupRows::Compact64 { signed, rows } => {
                let key = key.and_then(|key| Self::encode64(key, *signed).ok());
                let unequal = if self.inequality {
                    unequal.and_then(|unequal| Self::encode64(unequal, *signed).ok())
                } else {
                    None
                };
                Self::exists_in(rows, key, unequal)
            }
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
        for mut lookup in [
            JoinExistsLookup::new(false),
            JoinExistsLookup::new_compact64(false, false),
        ] {
            lookup.insert(None, key(1)).unwrap();
            lookup
                .insert(key(1), Some(JoinExistsKey::Signed(i128::MAX)))
                .unwrap();
            lookup.insert(key(1), None).unwrap();
            assert!(lookup.exists(key(1), None));
            assert!(!lookup.exists(None, key(1)));
            assert!(!lookup.exists(key(2), None));
        }
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
        for mut lookup in [
            JoinExistsLookup::new(true),
            JoinExistsLookup::new_compact64(true, false),
        ] {
            for value in 0..MAX_KEYS {
                lookup.insert(key(value as u128), key(1)).unwrap();
            }
            assert!(lookup.insert(key(0), key(2)).is_ok());
            assert!(lookup.exists(key(0), key(1)));
            assert!(lookup.insert(key(MAX_KEYS as u128), key(1)).is_err());
            assert!(!lookup.exists(key(MAX_KEYS as u128), key(2)));
        }
    }

    #[test]
    fn compact_unsigned64_lookup_matches_general_duplicate_and_null_semantics() {
        let values = [None, key(0), key(1), key(u64::MAX as u128)];
        for equality_rows in values {
            for unequal_rows in values {
                let mut general = JoinExistsLookup::new(true);
                let mut compact = JoinExistsLookup::new_compact64(true, false);
                for (equality, unequal) in [
                    (equality_rows, unequal_rows),
                    (equality_rows, key(7)),
                    (key(2), key(3)),
                ] {
                    general.insert(equality, unequal).unwrap();
                    compact.insert(equality, unequal).unwrap();
                }
                for equality in values {
                    for unequal in values {
                        assert_eq!(
                            compact.exists(equality, unequal),
                            general.exists(equality, unequal),
                            "equality={equality:?} inequality={unequal:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn compact_signed64_lookup_keeps_signed_boundaries() {
        let mut lookup = JoinExistsLookup::new_compact64(true, true);
        let min = Some(JoinExistsKey::Signed(i64::MIN as i128));
        let max = Some(JoinExistsKey::Signed(i64::MAX as i128));
        lookup.insert(min, max).unwrap();
        assert!(lookup.exists(min, min));
        assert!(!lookup.exists(min, max));
        assert!(!lookup.exists(max, min));
    }

    #[test]
    fn compact_lookup_rejects_values_outside_its_declared_key_width() {
        let mut unsigned = JoinExistsLookup::new_compact64(false, false);
        assert!(
            unsigned
                .insert(Some(JoinExistsKey::Unsigned(u64::MAX as u128 + 1)), None)
                .is_err()
        );
        assert!(
            unsigned
                .insert(Some(JoinExistsKey::Signed(1)), None)
                .is_err()
        );
        assert!(!unsigned.exists(Some(JoinExistsKey::Signed(1)), None));

        let mut signed = JoinExistsLookup::new_compact64(false, true);
        assert!(
            signed
                .insert(Some(JoinExistsKey::Signed(i64::MAX as i128 + 1)), None)
                .is_err()
        );
        assert!(
            signed
                .insert(Some(JoinExistsKey::Unsigned(1)), None)
                .is_err()
        );
        assert!(!signed.exists(Some(JoinExistsKey::Unsigned(1)), None));
    }
}
