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

type CandidatePair<K> = [Option<Candidates<K>>; 2];

enum PairLookupRows {
    General(HashMap<JoinExistsKey, CandidatePair<JoinExistsKey>>),
    Compact64 {
        signed: bool,
        rows: HashMap<u64, CompactCandidatePair>,
    },
}

#[derive(Clone, Copy)]
#[repr(u8)]
enum CompactCandidateState {
    Empty,
    Any,
    One,
    Multiple,
}

#[derive(Clone, Copy)]
struct CompactCandidatePair {
    values: [u64; 2],
    states: [CompactCandidateState; 2],
}

impl CompactCandidatePair {
    fn new() -> Self {
        Self {
            values: [0; 2],
            states: [CompactCandidateState::Empty; 2],
        }
    }

    fn insert(&mut self, slot: usize, value: Candidates<u64>) {
        match (self.states[slot], value) {
            (CompactCandidateState::Empty, Candidates::Any) => {
                self.states[slot] = CompactCandidateState::Any;
            }
            (CompactCandidateState::Empty, Candidates::One(value)) => {
                self.values[slot] = value;
                self.states[slot] = CompactCandidateState::One;
            }
            (CompactCandidateState::Any | CompactCandidateState::Multiple, _) => {}
            (CompactCandidateState::One, Candidates::Any) => {
                self.states[slot] = CompactCandidateState::Any;
            }
            (CompactCandidateState::One, Candidates::One(value)) => {
                if self.values[slot] != value {
                    self.states[slot] = CompactCandidateState::Multiple;
                }
            }
            (CompactCandidateState::Empty, Candidates::Multiple)
            | (CompactCandidateState::One, Candidates::Multiple) => {
                self.states[slot] = CompactCandidateState::Multiple;
            }
        }
    }

    fn matches(&self, slot: usize, unequal: Option<u64>) -> bool {
        match self.states[slot] {
            CompactCandidateState::Empty => false,
            CompactCandidateState::Any => true,
            CompactCandidateState::One => unequal.is_some_and(|value| value != self.values[slot]),
            CompactCandidateState::Multiple => unequal.is_some(),
        }
    }
}

/// A bounded lookup for rows that already passed their inner predicates.
/// A missing key represents SQL NULL. Callers must validate matching types.
pub struct JoinExistsLookup {
    inequality: bool,
    rows: LookupRows,
}

/// Two correlated lookups sharing one equality key map.
pub(crate) struct JoinExistsLookupPair {
    inequalities: [bool; 2],
    keys: [usize; 2],
    rows: PairLookupRows,
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
        Self::candidate_matches(*candidates, unequal)
    }

    fn candidate_matches<K: Copy + Eq>(candidates: Candidates<K>, unequal: Option<K>) -> bool {
        match candidates {
            Candidates::Any => true,
            Candidates::One(first) => unequal.is_some_and(|value| value != first),
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

impl JoinExistsLookupPair {
    pub(crate) fn new(inequalities: [bool; 2]) -> Self {
        Self {
            inequalities,
            keys: [0; 2],
            rows: PairLookupRows::General(HashMap::new()),
        }
    }

    /// Use only when all present correlation keys are signed or unsigned integers up to 64 bits.
    pub(crate) fn new_compact64(inequalities: [bool; 2], signed: bool) -> Self {
        Self {
            inequalities,
            keys: [0; 2],
            rows: PairLookupRows::Compact64 {
                signed,
                rows: HashMap::new(),
            },
        }
    }

    fn merge_candidate<K: Copy + Eq>(
        candidate: &mut Option<Candidates<K>>,
        next: Candidates<K>,
    ) -> bool {
        match candidate {
            None => {
                *candidate = Some(next);
                true
            }
            Some(Candidates::Any | Candidates::Multiple) => false,
            Some(Candidates::One(first)) => {
                if let Candidates::One(value) = next {
                    if *first != value {
                        *candidate = Some(Candidates::Multiple);
                    }
                }
                false
            }
        }
    }

    fn insert_pair_candidate<K: Copy + Eq + std::hash::Hash>(
        rows: &mut HashMap<K, CandidatePair<K>>,
        keys: &mut [usize; 2],
        slot: usize,
        key: K,
        value: Candidates<K>,
    ) -> Result<(), ExecutionError> {
        let Some(count) = keys.get_mut(slot) else {
            return Err(ExecutionError::Invalid(
                "JOIN EXISTS shared filter slot is out of range".into(),
            ));
        };
        if let Some(candidates) = rows.get_mut(&key) {
            if Self::merge_candidate(&mut candidates[slot], value) {
                if *count >= MAX_KEYS {
                    candidates[slot] = None;
                    return Err(ExecutionError::Invalid(
                        "JOIN EXISTS supports at most 200000 keys".into(),
                    ));
                }
                *count += 1;
            }
        } else {
            if *count >= MAX_KEYS {
                return Err(ExecutionError::Invalid(
                    "JOIN EXISTS supports at most 200000 keys".into(),
                ));
            }
            let mut candidates = [None, None];
            candidates[slot] = Some(value);
            rows.insert(key, candidates);
            *count += 1;
        }
        Ok(())
    }

    fn exists_in_pair<K: Copy + Eq + std::hash::Hash>(
        rows: &HashMap<K, CandidatePair<K>>,
        slot: usize,
        key: Option<K>,
        unequal: Option<K>,
    ) -> bool {
        let Some(key) = key else {
            return false;
        };
        rows.get(&key)
            .and_then(|candidates| candidates.get(slot))
            .and_then(|candidates| *candidates)
            .is_some_and(|candidates| JoinExistsLookup::candidate_matches(candidates, unequal))
    }

    fn insert_compact_candidate(
        rows: &mut HashMap<u64, CompactCandidatePair>,
        keys: &mut [usize; 2],
        slot: usize,
        key: u64,
        value: Candidates<u64>,
    ) -> Result<(), ExecutionError> {
        let Some(count) = keys.get_mut(slot) else {
            return Err(ExecutionError::Invalid(
                "JOIN EXISTS shared filter slot is out of range".into(),
            ));
        };
        if let Some(candidates) = rows.get_mut(&key) {
            if matches!(candidates.states[slot], CompactCandidateState::Empty) {
                if *count >= MAX_KEYS {
                    return Err(ExecutionError::Invalid(
                        "JOIN EXISTS supports at most 200000 keys".into(),
                    ));
                }
                *count += 1;
            }
            candidates.insert(slot, value);
        } else {
            if *count >= MAX_KEYS {
                return Err(ExecutionError::Invalid(
                    "JOIN EXISTS supports at most 200000 keys".into(),
                ));
            }
            let mut candidates = CompactCandidatePair::new();
            candidates.insert(slot, value);
            rows.insert(key, candidates);
            *count += 1;
        }
        Ok(())
    }

    /// Add a row to one of the two independent filtered lookups.
    pub(crate) fn insert(
        &mut self,
        slot: usize,
        key: Option<JoinExistsKey>,
        unequal: Option<JoinExistsKey>,
    ) -> Result<(), ExecutionError> {
        let Some(inequality) = self.inequalities.get(slot).copied() else {
            return Err(ExecutionError::Invalid(
                "JOIN EXISTS shared filter slot is out of range".into(),
            ));
        };
        let Some(key) = key else {
            return Ok(());
        };
        let value = if inequality {
            let Some(unequal) = unequal else {
                return Ok(());
            };
            Candidates::One(unequal)
        } else {
            Candidates::Any
        };
        match &mut self.rows {
            PairLookupRows::General(rows) => {
                Self::insert_pair_candidate(rows, &mut self.keys, slot, key, value)
            }
            PairLookupRows::Compact64 { signed, rows } => {
                let key = JoinExistsLookup::encode64(key, *signed)?;
                let value = match value {
                    Candidates::Any => Candidates::Any,
                    Candidates::One(unequal) => {
                        Candidates::One(JoinExistsLookup::encode64(unequal, *signed)?)
                    }
                    Candidates::Multiple => unreachable!("new lookup candidates are singular"),
                };
                Self::insert_compact_candidate(rows, &mut self.keys, slot, key, value)
            }
        }
    }

    /// Check one of the two correlated predicates without duplicating its key map.
    pub(crate) fn exists(
        &self,
        slot: usize,
        key: Option<JoinExistsKey>,
        unequal: Option<JoinExistsKey>,
    ) -> bool {
        let Some(inequality) = self.inequalities.get(slot).copied() else {
            return false;
        };
        match &self.rows {
            PairLookupRows::General(rows) => Self::exists_in_pair(rows, slot, key, unequal),
            PairLookupRows::Compact64 { signed, rows } => {
                let key = key.and_then(|key| JoinExistsLookup::encode64(key, *signed).ok());
                let unequal = if inequality {
                    unequal.and_then(|key| JoinExistsLookup::encode64(key, *signed).ok())
                } else {
                    None
                };
                let Some(key) = key else {
                    return false;
                };
                rows.get(&key)
                    .is_some_and(|candidates| candidates.matches(slot, unequal))
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
    fn paired_lookups_share_keys_but_keep_filter_candidates_independent() {
        for mut lookup in [
            JoinExistsLookupPair::new_compact64([true, true], false),
            JoinExistsLookupPair::new([true, true]),
        ] {
            lookup.insert(0, key(1), key(10)).unwrap();
            lookup.insert(0, key(1), key(20)).unwrap();
            lookup.insert(1, key(1), key(10)).unwrap();
            lookup.insert(1, key(2), key(20)).unwrap();

            assert!(lookup.exists(0, key(1), key(30)));
            assert!(lookup.exists(0, key(1), key(10)));
            assert!(!lookup.exists(1, key(1), key(10)));
            assert!(lookup.exists(1, key(1), key(20)));
            assert!(!lookup.exists(1, key(2), key(20)));
            assert!(!lookup.exists(0, None, key(20)));
            assert!(!lookup.exists(2, key(1), key(20)));
            assert!(lookup.insert(2, None, None).is_err());
        }
    }

    #[test]
    fn paired_lookup_without_inequality_preserves_any_match_semantics() {
        let mut lookup = JoinExistsLookupPair::new([false, true]);
        lookup.insert(0, key(7), None).unwrap();
        lookup.insert(1, key(7), key(2)).unwrap();

        assert!(lookup.exists(0, key(7), None));
        assert!(lookup.exists(0, key(7), key(2)));
        assert!(lookup.exists(1, key(7), key(3)));
        assert!(!lookup.exists(1, key(7), key(2)));
        assert!(!lookup.exists(1, key(7), None));
    }

    #[test]
    fn compact_paired_candidates_use_less_space_than_two_separate_summaries() {
        assert!(
            std::mem::size_of::<CompactCandidatePair>()
                < 2 * std::mem::size_of::<Candidates<u64>>()
        );
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
    fn paired_lookup_enforces_the_key_limit_per_filter() {
        let mut lookup = JoinExistsLookupPair::new_compact64([true, true], false);
        for value in 0..MAX_KEYS {
            lookup.insert(0, key(value as u128), key(1)).unwrap();
            lookup.insert(1, key(value as u128), key(2)).unwrap();
        }

        assert!(lookup.insert(0, key(MAX_KEYS as u128), key(1)).is_err());
        assert!(lookup.insert(1, key(MAX_KEYS as u128), key(2)).is_err());
        assert!(lookup.exists(0, key(0), key(2)));
        assert!(lookup.exists(1, key(0), key(1)));
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
