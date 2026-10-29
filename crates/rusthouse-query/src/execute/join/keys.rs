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
    // Preserve the backing allocation of repeated text probe keys. Rebuilding
    // the key vector for every fact row is especially costly in skewed joins.
    for (position, &index) in indices.iter().enumerate() {
        if !key_eligible(&row[index]) {
            result.clear();
            return false;
        }
        match &row[index] {
            typed::Cell::Text(value) => put_text_key(result, position, value),
            value => put_key(result, position, join_key(value)),
        }
    }
    result.truncate(indices.len());
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
#[path = "keys/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "keys/uuid_join_key_tests.rs"]
mod uuid_join_key_tests;

#[cfg(test)]
#[path = "keys/direct_numeric_join_probe_tests.rs"]
mod direct_numeric_join_probe_tests;

#[cfg(test)]
#[path = "keys/reusable_materialized_text_probe_tests.rs"]
mod reusable_materialized_text_probe_tests;
