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

pub(super) fn row_key(row: &[typed::Cell], indices: &[usize]) -> Option<Vec<typed::Key>> {
    let mut result = Vec::with_capacity(indices.len());
    for &index in indices {
        let cell = &row[index];
        if matches!(cell, typed::Cell::Null)
            || matches!(cell, typed::Cell::Numeric(Number::Float(value)) if value.is_nan())
        {
            return None;
        }
        result.push(cell.key());
    }
    Some(result)
}

pub(super) fn batch_key_into(
    batch: &[TypedColumn],
    row: usize,
    indices: &[usize],
    result: &mut Vec<typed::Key>,
) -> bool {
    result.clear();
    for &index in indices {
        let value = typed::cell(&batch[index], row);
        if matches!(value, typed::Cell::Null)
            || matches!(value, typed::Cell::Numeric(Number::Float(number)) if number.is_nan())
        {
            result.clear();
            return false;
        }
        result.push(value.key());
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
