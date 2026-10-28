//! Streaming approximate heavy hitters with bounded memory and O(log K)
//! updates. Slots keep stable IDs while an indexed min-heap tracks the current
//! smallest counter. Unseen keys replace the smallest SpaceSaving counter.
//! A hash-to-slot map probes borrowed String bytes without allocating on hits.
use super::predicates::{
    compile_numbers_predicates, monotonic_range_indices, number_matches_all,
};
use super::typed;
use super::{ExecutionError, Number, number_at, write_header};
use crate::{Projection, Select};
use rusthouse_storage::{TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

const MIN_CAPACITY: usize = 256;
const MAX_CAPACITY: usize = 2048;
const MAX_TEXT_BYTES: usize = 4096;

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
enum Key {
    Unsigned(u128),
    Signed(i128),
    Float(u64),
    Text(Vec<u8>),
    Uuid([u8; 16]),
}

pub(super) enum View<'a> {
    Unsigned(u128),
    Signed(i128),
    Float(u64),
    Text(&'a [u8]),
    Uuid(&'a [u8; 16]),
}

impl View<'_> {
    fn from_number(value: Number) -> Self {
        match value {
            Number::Unsigned(value) => Self::Unsigned(value),
            Number::Signed(value) => Self::Signed(value),
            Number::Float(value) => Self::Float(if value == 0.0 {
                0
            } else if value.is_nan() {
                f64::NAN.to_bits()
            } else {
                value.to_bits()
            }),
        }
    }

    fn owned(&self) -> Key {
        match self {
            Self::Unsigned(value) => Key::Unsigned(*value),
            Self::Signed(value) => Key::Signed(*value),
            Self::Float(value) => Key::Float(*value),
            Self::Text(value) => Key::Text(value.to_vec()),
            Self::Uuid(value) => Key::Uuid(**value),
        }
    }

    fn equals(&self, other: &Key) -> bool {
        match (self, other) {
            (Self::Unsigned(a), Key::Unsigned(b)) => a == b,
            (Self::Signed(a), Key::Signed(b)) => a == b,
            (Self::Float(a), Key::Float(b)) => a == b,
            (Self::Text(a), Key::Text(b)) => *a == b.as_slice(),
            (Self::Uuid(a), Key::Uuid(b)) => **a == *b,
            _ => false,
        }
    }

    fn fingerprint(&self) -> u64 {
        let (tag, bytes): (u64, &[u8]) = match self {
            Self::Unsigned(value) => return fingerprint_bytes(1, &value.to_le_bytes()),
            Self::Signed(value) => return fingerprint_bytes(2, &value.to_le_bytes()),
            Self::Float(value) => return fingerprint_bytes(3, &value.to_le_bytes()),
            Self::Text(bytes) => (4, bytes),
            Self::Uuid(bytes) => (5, bytes.as_slice()),
        };
        fingerprint_bytes(tag, bytes)
    }
}

fn fingerprint_bytes(tag: u64, value: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64 ^ tag;
    for &byte in value {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xff51afd7ed558ccd);
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xc4ceb9fe1a85ec53);
    hash ^ (hash >> 33)
}

pub(super) fn view_at(column: &TypedColumn, row: usize) -> Result<Option<View<'_>>, ExecutionError> {
    match column {
        TypedColumn::Numeric(values) => Ok(Some(View::from_number(number_at(values, row)))),
        TypedColumn::NullableNumeric { values, nulls } if nulls[row] == 0 => {
            Ok(Some(View::from_number(number_at(values, row))))
        }
        TypedColumn::Text(values) => Ok(Some(View::Text(values.get(row).ok_or_else(|| {
            ExecutionError::Invalid("missing topK String value".into())
        })?))),
        TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => {
            Ok(Some(View::Text(values.get(row).ok_or_else(|| {
                ExecutionError::Invalid("missing topK nullable String value".into())
            })?)))
        }
        TypedColumn::Uuid(values) => Ok(Some(View::Uuid(values[row].as_bytes()))),
        TypedColumn::NullableNumeric { .. } | TypedColumn::NullableText { .. } => Ok(None),
    }
}

struct Slot {
    key: Key,
    hash: u64,
    frequency: u64,
}

pub(super) struct SpaceSaving {
    k: usize,
    capacity: usize,
    slots: Vec<Slot>,
    heap: Vec<usize>,
    positions: Vec<usize>,
    by_hash: HashMap<u64, Vec<usize>>,
}

impl SpaceSaving {
    pub(super) fn new(k: usize) -> Self {
        let capacity = (k * 16).clamp(MIN_CAPACITY, MAX_CAPACITY);
        Self {
            k,
            capacity,
            slots: Vec::with_capacity(capacity),
            heap: Vec::with_capacity(capacity),
            positions: Vec::with_capacity(capacity),
            by_hash: HashMap::new(),
        }
    }

    fn less(&self, left: usize, right: usize) -> bool {
        let a = self.heap[left];
        let b = self.heap[right];
        self.slots[a].frequency < self.slots[b].frequency
            || (self.slots[a].frequency == self.slots[b].frequency && a < b)
    }

    fn swap(&mut self, a: usize, b: usize) {
        self.heap.swap(a, b);
        self.positions[self.heap[a]] = a;
        self.positions[self.heap[b]] = b;
    }

    fn sift_up(&mut self, mut index: usize) {
        while index > 0 {
            let parent = (index - 1) / 2;
            if !self.less(index, parent) {
                break;
            }
            self.swap(index, parent);
            index = parent;
        }
    }

    fn sift_down(&mut self, mut index: usize) {
        loop {
            let left = index * 2 + 1;
            if left >= self.heap.len() {
                break;
            }
            let right = left + 1;
            let smallest = if right < self.heap.len() && self.less(right, left) {
                right
            } else {
                left
            };
            if !self.less(smallest, index) {
                break;
            }
            self.swap(index, smallest);
            index = smallest;
        }
    }

    pub(super) fn observe(&mut self, key: View<'_>) -> Result<(), ExecutionError> {
        if matches!(&key, View::Text(value) if value.len() > MAX_TEXT_BYTES) {
            return Err(ExecutionError::Invalid(
                "topK String key exceeds 4096-byte budget".into(),
            ));
        }
        let hash = key.fingerprint();
        let existing = self.by_hash.get(&hash).and_then(|bucket| {
            bucket
                .iter()
                .copied()
                .find(|&index| key.equals(&self.slots[index].key))
        });
        if let Some(index) = existing {
            self.slots[index].frequency = self.slots[index]
                .frequency
                .checked_add(1)
                .ok_or_else(|| ExecutionError::Invalid("topK frequency overflow".into()))?;
            self.sift_down(self.positions[index]);
            return Ok(());
        }
        let owned = key.owned();
        if self.slots.len() < self.capacity {
            let index = self.slots.len();
            self.slots.push(Slot {
                key: owned,
                hash,
                frequency: 1,
            });
            self.by_hash.entry(hash).or_default().push(index);
            let position = self.heap.len();
            self.heap.push(index);
            self.positions.push(position);
            self.sift_up(position);
            return Ok(());
        }
        // SpaceSaving replaces the minimum-frequency key and increments
        // its counter. Heap positions and slot identities remain stable.
        let victim = self.heap[0];
        let old_hash = self.slots[victim].hash;
        let bucket = self.by_hash.get_mut(&old_hash).expect("heap slot is indexed");
        bucket.retain(|&index| index != victim);
        if bucket.is_empty() {
            self.by_hash.remove(&old_hash);
        }
        let frequency = self.slots[victim]
            .frequency
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("topK frequency overflow".into()))?;
        self.slots[victim] = Slot {
            key: owned,
            hash,
            frequency,
        };
        self.by_hash.entry(hash).or_default().push(victim);
        self.sift_down(0);
        Ok(())
    }

    pub(super) fn ranked(&self) -> Vec<&Key> {
        let mut slots = self.slots.iter().collect::<Vec<_>>();
        slots.sort_unstable_by(|left, right| {
            right
                .frequency
                .cmp(&left.frequency)
                .then_with(|| left.key.cmp(&right.key))
        });
        slots.into_iter().take(self.k).map(|slot| &slot.key).collect()
    }
}

pub(super) fn write_key<W: Write>(output: &mut W, key: &Key) -> Result<(), ExecutionError> {
    match key {
        Key::Unsigned(value) => write!(output, "{value}")?,
        Key::Signed(value) => write!(output, "{value}")?,
        Key::Float(value) => write!(output, "{}", f64::from_bits(*value))?,
        Key::Text(value) => {
            output.write_all(b"'")?;
            for byte in value {
                match *byte {
                    b'\n' => output.write_all(b"\\n")?,
                    b'\r' => output.write_all(b"\\r")?,
                    b'\t' => output.write_all(b"\\t")?,
                    b'\\' => output.write_all(b"\\\\")?,
                    b'\'' => output.write_all(b"\\'")?,
                    0..=31 => write!(output, "\\x{byte:02X}")?,
                    _ => output.write_all(std::slice::from_ref(byte))?,
                }
            }
            output.write_all(b"'")?;
        }
        Key::Uuid(value) => {
            output.write_all(b"'")?;
            for (index, byte) in value.iter().enumerate() {
                if matches!(index, 4 | 6 | 8 | 10) {
                    output.write_all(b"-")?;
                }
                write!(output, "{byte:02x}")?;
            }
            output.write_all(b"'")?;
        }
    }
    Ok(())
}

fn output_result<W: Write>(
    query: &Select,
    output: &mut W,
    column: &str,
    k: usize,
    keys: &[&Key],
) -> Result<(), ExecutionError> {
    write_header(query, output, &format!("topK({k})({column})"))?;
    if query.limit == Some(0) || query.offset.unwrap_or(0) > 0 {
        return Ok(());
    }
    output.write_all(b"[")?;
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            output.write_all(b",")?;
        }
        write_key(output, key)?;
    }
    output.write_all(b"]\n")?;
    Ok(())
}

pub(super) fn execute_part<W: Write>(
    query: &Select,
    part: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::TopK { column, k } = &query.projection else {
        unreachable!("topK executor requires topK projection");
    };
    let metadata = inspect_query_source(part)?;
    if !metadata.columns.iter().any(|candidate| candidate.name == *column) {
        return Err(ExecutionError::Invalid(format!("unknown topK column: {column}")));
    }
    let mut names = vec![column.clone()];
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        if !names.iter().any(|name| name == predicate.column()) {
            names.push(predicate.column().to_owned());
        }
    }
    let indices: HashMap<String, usize> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect();
    let types: HashMap<String, String> = metadata
        .columns
        .iter()
        .map(|candidate| (candidate.name.clone(), candidate.data_type.clone()))
        .collect();
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    let mut scan = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let mut top = SpaceSaving::new(*k);
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let selected = indices[column];
    while let Some(batch) = scan.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            if let Some(value) = view_at(&batch[selected], row)? {
                top.observe(value)?;
            }
        }
    }
    output_result(query, output, column, *k, &top.ranked())
}

pub(super) fn execute_numbers<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let Projection::TopK { column, k } = &query.projection else {
        unreachable!("generated topK requires topK projection");
    };
    if column != "number" {
        return Err(ExecutionError::Invalid(
            "numbers(N) exposes only the number column".into(),
        ));
    }
    let predicates = compile_numbers_predicates(query)?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    let direct = if no_filters {
        Some((0, rows))
    } else {
        monotonic_range_indices(&predicates, start, rows, step)
    };
    if let Some((lower, upper)) = direct {
        let keys = (lower..upper)
            .take(*k)
            .map(|index| Key::Unsigned(
                u128::from(start) + u128::from(index) * u128::from(step),
            ))
            .collect::<Vec<_>>();
        let refs = keys.iter().collect::<Vec<_>>();
        return output_result(query, output, column, *k, &refs);
    }
    if rows > 10_000_000 {
        return Err(ExecutionError::Invalid(
            "large generated topK requires monotonic WHERE predicates".into(),
        ));
    }
    let mut top = SpaceSaving::new(*k);
    for index in 0..rows {
        let number = u128::from(start) + u128::from(index) * u128::from(step);
        if number_matches_all(Number::Unsigned(number), &predicates) {
            top.observe(View::Unsigned(number))?;
        }
    }
    output_result(query, output, column, *k, &top.ranked())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_heap(sketch: &SpaceSaving) {
        for (position, &id) in sketch.heap.iter().enumerate() {
            assert_eq!(sketch.positions[id], position);
            if position > 0 {
                assert!(!sketch.less(position, (position - 1) / 2));
            }
        }
        assert!(sketch.slots.len() <= sketch.capacity);
    }

    #[test]
    fn heavy_hitters_survive_large_unique_noise() {
        let mut top = SpaceSaving::new(3);
        for value in 0..100_000u128 {
            top.observe(View::Unsigned(10_000 + value)).unwrap();
        }
        for _ in 0..20_000 {
            top.observe(View::Unsigned(1)).unwrap();
        }
        for _ in 0..15_000 {
            top.observe(View::Unsigned(2)).unwrap();
        }
        for _ in 0..10_000 {
            top.observe(View::Unsigned(3)).unwrap();
        }
        assert_eq!(
            top.ranked(),
            vec![&Key::Unsigned(1), &Key::Unsigned(2), &Key::Unsigned(3)],
        );
        assert_heap(&top);
    }

    #[test]
    fn bounded_slots_and_update_heap_positions() {
        let mut top = SpaceSaving::new(5);
        for value in 0..5000u128 {
            top.observe(View::Unsigned(value)).unwrap();
        }
        assert_heap(&top);
        for _ in 0..100 {
            top.observe(View::Unsigned(4999)).unwrap();
        }
        assert_heap(&top);
    }
}
