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

fn join_key(value: &typed::Cell) -> typed::Key {
    match value {
        typed::Cell::Uuid(text) => typed::Key::UuidBytes(typed::uuid_key(text)),
        _ => value.key(),
    }
}

fn key_eligible(value: &typed::Cell) -> bool {
    !matches!(value, typed::Cell::Null)
        && !matches!(value, typed::Cell::Numeric(Number::Float(number)) if number.is_nan())
}

pub(super) fn row_key(row: &[typed::Cell], indices: &[usize]) -> Option<Vec<typed::Key>> {
    // Reject invalid later components before cloning any variable-width keys.
    if indices.iter().any(|&index| !key_eligible(&row[index])) {
        return None;
    }
    Some(indices.iter().map(|&index| join_key(&row[index])).collect())
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

pub(super) fn batch_key_into(
    batch: &[TypedColumn],
    row: usize,
    indices: &[usize],
    result: &mut Vec<typed::Key>,
) -> bool {
    result.clear();
    // A later NULL must not force us to clone an earlier String/UUID key.
    if indices.len() > 1
        && indices
            .iter()
            .any(|&index| typed::is_null_at(&batch[index], row))
    {
        return false;
    }
    // The same vector is reused across batches and rows.
    result.reserve(indices.len());
    for &index in indices {
        // UUID probe keys are already stored as canonical bytes. Avoid
        // allocating a 36-byte display String for each probed row.
        let key = match &batch[index] {
            TypedColumn::Uuid(values) => typed::Key::UuidBytes(*values[row].as_bytes()),
            TypedColumn::Numeric(values) => {
                let value = super::super::number_at(values, row);
                if matches!(value, Number::Float(number) if number.is_nan()) {
                    result.clear();
                    return false;
                }
                typed::Key::Numeric(super::super::NumberKey::from(value))
            }
            TypedColumn::NullableNumeric { values, nulls } => {
                if nulls[row] != 0 {
                    result.clear();
                    return false;
                }
                let value = super::super::number_at(values, row);
                if matches!(value, Number::Float(number) if number.is_nan()) {
                    result.clear();
                    return false;
                }
                typed::Key::Numeric(super::super::NumberKey::from(value))
            }
            column => {
                let value = typed::cell(column, row);
                if !key_eligible(&value) {
                    result.clear();
                    return false;
                }
                match value {
                    typed::Cell::Text(bytes) => typed::Key::Text(bytes),
                    _ => value.key(),
                }
            }
        };
        result.push(key);
    }
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
