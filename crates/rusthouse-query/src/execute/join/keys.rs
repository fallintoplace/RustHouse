use super::*;

pub(super) fn base_type(value: &str) -> &str {
    value
        .strip_prefix("Nullable(")
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(value)
}

pub(super) fn resolve(
    query: &JoinSelect,
    column: &JoinColumn,
    left_names: &[String],
    right_names: &[String],
) -> Result<usize, ExecutionError> {
    let (names, start) = if column.relation == query.left_alias {
        (left_names, 0)
    } else if column.relation == query.right_alias {
        (right_names, left_names.len())
    } else {
        return Err(invalid(format!(
            "unknown JOIN qualifier: {}",
            column.relation
        )));
    };
    names
        .iter()
        .position(|name| name == &column.name)
        .map(|index| start + index)
        .ok_or_else(|| {
            invalid(format!(
                "unknown JOIN column: {}.{}",
                column.relation, column.name
            ))
        })
}

fn numeric_join_key(number: Number) -> super::super::NumberKey {
    // Integer JOIN comparison is by numeric value, not source signedness.
    match number {
        Number::Signed(value) if value >= 0 =>
            super::super::NumberKey::Unsigned(value as u128),
        other => super::super::NumberKey::from(other),
    }
}

fn join_key(value: &typed::Cell) -> typed::Key {
    match value {
        typed::Cell::Uuid(text) => typed::Key::UuidBytes(typed::uuid_key(text)),
        typed::Cell::Numeric(number) =>
            typed::Key::Numeric(numeric_join_key(*number)),
        _ => value.key(),
    }
}

fn key_eligible(value: &typed::Cell) -> bool {
    !matches!(value, typed::Cell::Null)
        && !matches!(value, typed::Cell::Numeric(Number::Float(number)) if number.is_nan())
}

pub(super) fn row_key(row: &[typed::Cell], indices: &[usize]) -> Option<Vec<typed::Key>> {
    let mut key = Vec::with_capacity(indices.len());
    row_key_into(row, indices, &mut key).then_some(key)
}

pub(super) fn row_key_into(
    row: &[typed::Cell],
    indices: &[usize],
    result: &mut Vec<typed::Key>,
) -> bool {
    result.clear();
    if indices.iter().any(|&index| !key_eligible(&row[index])) {
        return false;
    }
    result.extend(indices.iter().map(|&index| join_key(&row[index])));
    true
}

// Replace scalar keys in place. More importantly, preserve the capacity of
// already decoded String probe keys across rows of the same JOIN batch.
fn put_key(result: &mut Vec<typed::Key>, position: usize, key: typed::Key) {
    if position < result.len() {
        result[position] = key;
    } else {
        result.push(key);
    }
}

fn put_text_key(result: &mut Vec<typed::Key>, position: usize, bytes: &[u8]) {
    if let Some(typed::Key::Text(existing)) = result.get_mut(position) {
        existing.clear();
        existing.extend_from_slice(bytes);
    } else {
        put_key(result, position, typed::Key::Text(bytes.to_vec()));
    }
}

pub(super) fn batch_key_into(
    batch: &[TypedColumn],
    row: usize,
    indices: &[usize],
    result: &mut Vec<typed::Key>,
) -> bool {
    // Reject all NULL components before changing existing buffers.
    if indices
        .iter()
        .any(|&index| typed::is_null_at(&batch[index], row))
    {
        result.clear();
        return false;
    }
    result.reserve(indices.len().saturating_sub(result.len()));
    for (position, &index) in indices.iter().enumerate() {
        match &batch[index] {
            TypedColumn::Uuid(values) => {
                put_key(result, position, typed::Key::UuidBytes(*values[row].as_bytes()));
            }
            TypedColumn::Text(values) | TypedColumn::NullableText { values, .. } => {
                put_text_key(
                    result,
                    position,
                    values.get(row).expect("validated typed string row"),
                );
            }
            TypedColumn::Numeric(values) | TypedColumn::NullableNumeric { values, .. } => {
                let value = super::super::number_at(values, row);
                if matches!(value, Number::Float(number) if number.is_nan()) {
                    result.clear();
                    return false;
                }
                put_key(
                    result,
                    position,
                    typed::Key::Numeric(numeric_join_key(value)),
                );
            }
        }
    }
    result.truncate(indices.len());
    true
}

pub(super) fn cell_at<'a>(
    left_rows: &'a [Vec<typed::Cell>],
    right_rows: &'a [Vec<typed::Cell>],
    pair: &Pair,
    index: usize,
    left_width: usize,
) -> Option<&'a typed::Cell> {
    if index < left_width {
        pair.0.map(|left| &left_rows[left][index])
    } else {
        pair.1.map(|right| &right_rows[right][index - left_width])
    }
}

pub(super) fn add_pair(pairs: &mut Vec<Pair>, pair: Pair) -> Result<(), ExecutionError> {
    if pairs.len() >= MAX_RESULT_ROWS {
        return Err(invalid("JOIN produces more than 65536 rows"));
    }
    pairs.push(pair);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusthouse_storage::read_legacy_strings;

    #[test]
    fn row_key_probe_reuses_storage_and_rejects_null_keys() {
        let row = [
            typed::Cell::Numeric(Number::Unsigned(7)),
            typed::Cell::Text(b"abc".to_vec()),
        ];
        let mut probe = Vec::with_capacity(2);
        assert!(row_key_into(&row, &[0, 1], &mut probe));
        let capacity = probe.capacity();
        assert!(row_key_into(&row, &[0, 1], &mut probe));
        assert_eq!(probe.capacity(), capacity);
        assert_eq!(probe, row_key(&row, &[0, 1]).unwrap());
        let invalid = [typed::Cell::Null, typed::Cell::Text(b"abc".to_vec())];
        assert!(!row_key_into(&invalid, &[0, 1], &mut probe));
        assert!(probe.is_empty());
    }

    #[test]
    fn composite_probe_reserves_all_key_slots() {
        let values = vec![
            TypedColumn::Numeric(rusthouse_storage::NumericColumn::UInt8(vec![1])),
            TypedColumn::Numeric(rusthouse_storage::NumericColumn::UInt8(vec![2])),
            TypedColumn::Numeric(rusthouse_storage::NumericColumn::UInt8(vec![3])),
        ];
        let mut key = Vec::new();
        assert!(batch_key_into(&values, 0, &[0, 1, 2], &mut key));
        assert_eq!(key.len(), 3);
        assert!(key.capacity() >= 3);
    }

    #[test]
    fn repeated_text_probes_reuse_the_same_allocation() {
        let mut stream = b"\x04same\x04same\x04diff".as_slice();
        let strings = read_legacy_strings(&mut stream, 3).unwrap();
        let batch = vec![TypedColumn::Text(strings)];
        let mut key = Vec::new();
        assert!(batch_key_into(&batch, 0, &[0], &mut key));
        let (address, capacity) = match &key[0] {
            typed::Key::Text(bytes) => (bytes.as_ptr(), bytes.capacity()),
            _ => panic!("expected text key"),
        };
        for index in 1..3 {
            assert!(batch_key_into(&batch, index, &[0], &mut key));
            match &key[0] {
                typed::Key::Text(bytes) => {
                    assert_eq!(bytes.as_ptr(), address);
                    assert_eq!(bytes.capacity(), capacity);
                }
                _ => panic!("expected text key"),
            }
        }
        assert!(matches!(&key[0], typed::Key::Text(bytes) if bytes == b"diff"));
    }

    #[test]
    fn probe_key_moves_text_and_rejects_nullable_text() {
        let mut stream = b"\x03abc\x03def".as_slice();
        let values = read_legacy_strings(&mut stream, 2).unwrap();
        let batch = vec![TypedColumn::NullableText {
            values,
            nulls: vec![0, 1],
        }];
        let mut key = Vec::new();
        assert!(batch_key_into(&batch, 0, &[0], &mut key));
        assert!(matches!(key.as_slice(), [typed::Key::Text(value)] if value == b"abc"));
        assert!(!batch_key_into(&batch, 1, &[0], &mut key));
        assert!(key.is_empty());
    }
}

#[cfg(test)]
mod uuid_join_key_tests {
    use super::*;
    use rusthouse_storage::read_uuid_values;

    #[test]
    fn build_and_probe_share_raw_uuid_keys() {
        let mut disk = [0u8; 32];
        disk[16] = 1;
        let values = read_uuid_values(&mut disk.as_slice(), 2).unwrap();
        let rows = vec![
            typed::Cell::Uuid(values[0].to_string()),
            typed::Cell::Uuid(values[1].to_string()),
        ];
        let batch = [TypedColumn::Uuid(values)];
        let build = row_key(&rows[..1], &[0]).unwrap();
        let mut reusable = Vec::new();
        assert!(batch_key_into(&batch, 0, &[0], &mut reusable));
        assert!(matches!(reusable.first(), Some(typed::Key::UuidBytes(_))));
        assert!(build == reusable);
        assert!(batch_key_into(&batch, 1, &[0], &mut reusable));
        assert!(build != reusable);
        assert!(row_key_into(&rows[1..], &[0], &mut reusable));
        assert!(reusable == row_key(&rows[1..], &[0]).unwrap());
    }
}

#[cfg(test)]
mod direct_numeric_join_probe_tests {
    use super::*;
    use rusthouse_storage::NumericColumn;

    #[test]
    fn native_probe_keys_match_materialized_build_keys() {
        let batch = [
            TypedColumn::Numeric(NumericColumn::Int64(vec![-12, 9])),
            TypedColumn::NullableNumeric {
                values: NumericColumn::UInt64(vec![7, 8]),
                nulls: vec![0, 1],
            },
        ];
        let mut key = Vec::new();
        assert!(batch_key_into(&batch, 0, &[0, 1], &mut key));
        let row = [typed::Cell::Numeric(Number::Signed(-12)),
                   typed::Cell::Numeric(Number::Unsigned(7))];
        assert_eq!(key, row_key(&row, &[0, 1]).unwrap());
        assert!(!batch_key_into(&batch, 1, &[0, 1], &mut key));
        assert!(key.is_empty());
        let nan = [TypedColumn::Numeric(NumericColumn::Float64(vec![f64::NAN]))];
        assert!(!batch_key_into(&nan, 0, &[0], &mut key));
    }
}
