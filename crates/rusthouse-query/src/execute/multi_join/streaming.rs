//! Streaming, bounded-build execution for INNER join chains.
//! The fact relation is read in batches; only dimension-side build rows persist.

use super::*;
use rusthouse_storage::TypedQueryReader;

struct Stage {
    rows: Vec<Vec<typed::Cell>>,
    hash: HashMap<Vec<typed::Key>, Bucket>,
    left_keys: Vec<usize>,
    any: bool,
}

#[derive(Default)]
struct OutputState {
    seen: u64,
    emitted: usize,
}

struct OutputSpec<'a> {
    selected: &'a [(usize, String, Option<usize>)],
    null_filter: Option<(usize, bool)>,
    offset: usize,
    limit: Option<usize>,
}

fn visit<W: Write>(
    depth: usize,
    row: &mut Vec<typed::Cell>,
    stages: &[Stage],
    spec: &OutputSpec<'_>,
    state: &mut OutputState,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if spec.limit.is_some_and(|limit| state.emitted >= limit) {
        return Ok(());
    }
    if depth == stages.len() {
        if let Some((column, not_null)) = spec.null_filter {
            let is_null = matches!(row[column], typed::Cell::Null);
            if is_null == not_null {
                return Ok(());
            }
        }
        state.seen = state
            .seen
            .checked_add(1)
            .ok_or_else(|| invalid("JOIN matched-row count overflow"))?;
        // Keep the existing result cap for unpaged joins, but never retain
        // a large output vector for a LIMIT/OFFSET query.
        if spec.limit.is_none() && state.seen > MAX_RESULT_ROWS as u64 {
            return Err(invalid("JOIN produces more than 131072 rows"));
        }
        if state.seen <= spec.offset as u64 {
            return Ok(());
        }
        for (position, (index, _, fallback)) in spec.selected.iter().enumerate() {
            if position != 0 {
                output.write_all(b"\t")?;
            }
            let value = &row[*index];
            let value = if matches!(value, typed::Cell::Null) {
                fallback.and_then(|index| row.get(index)).unwrap_or(value)
            } else {
                value
            };
            typed::write_value(output, value)?;
        }
        output.write_all(b"\n")?;
        state.emitted += 1;
        return Ok(());
    }

    let stage = &stages[depth];
    let Some(key) = row_key(row, &stage.left_keys) else {
        return Ok(());
    };
    let Some(matches) = stage.hash.get(&key) else {
        return Ok(());
    };
    for &matched in matches.iter().take(if stage.any { 1 } else { usize::MAX }) {
        let original_len = row.len();
        row.extend_from_slice(&stage.rows[matched]);
        let result = visit(depth + 1, row, stages, spec, state, output);
        row.truncate(original_len);
        result?;
        if spec.limit.is_some_and(|limit| state.emitted >= limit) {
            break;
        }
    }
    Ok(())
}

pub(super) fn execute<W: Write>(
    query: &JoinSelect,
    paths: &[PathBuf],
    relations: &[Relation],
    initial_step: &JoinStep,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if relations[0].names.is_empty() {
        return Err(invalid("JOIN requires nonempty table schemas"));
    }
    let selected = output_columns(query, relations, initial_step)?;
    let null_filter = query
        .where_null
        .as_ref()
        .map(|(column, not_null)| {
            column_index(relations, column).map(|index| (index, *not_null))
        })
        .transpose()?;
    let spec = OutputSpec {
        selected: &selected,
        null_filter,
        offset: query.offset,
        limit: query.limit,
    };
    let mut stages = Vec::with_capacity(relations.len() - 1);
    let mut retained = 0usize;
    for (step_index, step) in std::iter::once(initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        let relation = &relations[step_index + 1];
        let (left_keys, right_keys) =
            step_keys(step, &relations[..=step_index + 1], relation.start)?;
        let available = MAX_CHAIN_BYTES.saturating_sub(retained);
        let right_rows = read_rows(
            &paths[step_index + 1],
            &relation.names,
            relation.rows,
            crate::join::MAX_STREAM_JOIN_BUILD_ROWS,
            available.min(MAX_STREAM_BUILD_BYTES),
        )?;
        for row in &right_rows {
            retained = retained
                .checked_add(retained_row_bytes(row)?)
                .ok_or_else(|| invalid("JOIN build size overflow"))?;
        }
        if retained > MAX_CHAIN_BYTES {
            return Err(invalid("JOIN exceeds its retained-row memory budget"));
        }
        let mut hash = HashMap::with_capacity(right_rows.len());
        for (index, row) in right_rows.iter().enumerate() {
            if let Some(key) = row_key(row, &right_keys) {
                hash.entry(key)
                    .and_modify(|bucket: &mut Bucket| bucket.push(index))
                    .or_insert(Bucket::One(index));
            }
        }
        stages.push(Stage {
            rows: right_rows,
            hash,
            left_keys,
            any: step.any,
        });
    }
    write_join_header(output, &selected)?;
    if stages.iter().any(|stage| stage.rows.is_empty()) {
        return Ok(());
    }
    // Retain the full logical row width so later join stages can address
    // their original absolute column indexes. Decode only columns actually
    // used by join keys, output projections, or the final NULL filter.
    let fact_width = relations[0].names.len();
    let mut needed = vec![false; fact_width];
    for stage in &stages {
        for &index in &stage.left_keys {
            if index < fact_width {
                needed[index] = true;
            }
        }
    }
    for &(index, _, fallback) in &selected {
        if index < fact_width {
            needed[index] = true;
        }
        if let Some(index) = fallback.filter(|&index| index < fact_width) {
            needed[index] = true;
        }
    }
    if let Some((index, _)) = null_filter {
        if index < fact_width {
            needed[index] = true;
        }
    }
    let mut projected = needed
        .iter()
        .enumerate()
        .filter_map(|(index, &used)| used.then_some(index))
        .collect::<Vec<_>>();
    if projected.is_empty() {
        // The typed reader requires at least one source stream to count rows.
        projected.push(0);
    }
    let projected_names = projected
        .iter()
        .map(|&index| relations[0].names[index].clone())
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(&paths[0], &projected_names)?;
    let mut scanned = 0u64;
    let mut state = OutputState::default();
    while let Some(batch) = reader.next_batch()? {
        for index in 0..batch[0].len() {
            scanned = scanned
                .checked_add(1)
                .ok_or_else(|| invalid("JOIN input row count overflow"))?;
            let mut row = vec![typed::Cell::Null; fact_width];
            for (position, &column) in projected.iter().enumerate() {
                row[column] = typed::cell(&batch[position], index);
            }
            visit(0, &mut row, &stages, &spec, &mut state, output)?;
            if spec.limit.is_some_and(|limit| state.emitted >= limit) {
                return Ok(());
            }
        }
    }
    if scanned != relations[0].rows {
        return Err(invalid("JOIN input row count changed during scan"));
    }
    Ok(())
}
