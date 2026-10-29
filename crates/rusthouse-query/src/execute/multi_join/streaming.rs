//! Streaming, bounded-build execution for INNER join chains.
//! The fact relation is read in batches; only dimension-side build rows persist.

use super::*;
use rusthouse_storage::TypedQueryReader;
use std::collections::hash_map::Entry;

use super::super::join::read_projected_build_rows;

// Conservatively charge hash buckets, key vectors and owned payloads as
// well as the already-accounted retained dimension rows.
fn hash_key_bytes(key: &[typed::Key]) -> Result<usize, ExecutionError> {
    let base = std::mem::size_of::<Vec<typed::Key>>()
        .checked_add(96)
        .and_then(|bytes| {
            key.len()
                .checked_mul(std::mem::size_of::<typed::Key>())
                .and_then(|slots| bytes.checked_add(slots))
        })
        .ok_or_else(|| invalid("JOIN hash key memory overflow"))?;
    key.iter().try_fold(base, |total, part| {
        let bytes = match part {
            typed::Key::Text(value) => value.len(),
            typed::Key::Uuid(value) => value.len(),
            _ => 0,
        };
        total
            .checked_add(bytes)
            .ok_or_else(|| invalid("JOIN hash key memory overflow"))
    })
}

fn charge_hash_build(retained: &mut usize, bytes: usize) -> Result<(), ExecutionError> {
    *retained = retained
        .checked_add(bytes)
        .filter(|&total| total <= MAX_CHAIN_BYTES)
        .ok_or_else(|| invalid("JOIN hash-build memory budget exceeded"))?;
    Ok(())
}

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
        // Output is written directly to the caller, not kept in an
        // intermediate row vector. No materialized-join result cap applies.
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
    // Project all dimension columns needed by any later stage, output
    // expression, or final filter. Preserve the original logical indexes.
    let width = relations
        .last()
        .map(|relation| relation.start + relation.names.len())
        .unwrap_or(0);
    let mut referenced = vec![false; width];
    for (step_index, step) in std::iter::once(initial_step)
        .chain(query.steps.iter())
        .enumerate()
    {
        let relation = &relations[step_index + 1];
        let (left, right) = step_keys(step, &relations[..=step_index + 1], relation.start)?;
        for index in left {
            referenced[index] = true;
        }
        for index in right {
            referenced[relation.start + index] = true;
        }
    }
    for &(index, _, fallback) in &selected {
        referenced[index] = true;
        if let Some(index) = fallback {
            referenced[index] = true;
        }
    }
    if let Some((index, _)) = null_filter {
        referenced[index] = true;
    }
    let projections = relations
        .iter()
        .map(|relation| {
            let mut positions = (0..relation.names.len())
                .filter(|&index| referenced[relation.start + index])
                .collect::<Vec<_>>();
            if positions.is_empty() && !relation.names.is_empty() {
                positions.push(0);
            }
            positions
        })
        .collect::<Vec<_>>();

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
        let right_rows = read_projected_build_rows(
            &paths[step_index + 1],
            &relation.names,
            &projections[step_index + 1],
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
        // A skewed build can contain far fewer distinct keys than input
        // rows. Reserve a small table, then charge each retained key.
        let mut hash = HashMap::with_capacity(right_rows.len().min(4096));
        for (index, row) in right_rows.iter().enumerate() {
            if let Some(key) = row_key(row, &right_keys) {
                match hash.entry(key) {
                    Entry::Vacant(entry) => {
                        charge_hash_build(&mut retained, hash_key_bytes(entry.key())?)?;
                        entry.insert(Bucket::One(index));
                    }
                    Entry::Occupied(mut entry) if !step.any => {
                        // Count a duplicate index and conservative Vec growth
                        // overhead. ANY only needs its first matching index.
                        charge_hash_build(
                            &mut retained,
                            std::mem::size_of::<usize>() * 2,
                        )?;
                        entry.get_mut().push(index);
                    }
                    Entry::Occupied(_) => {}
                }
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
    // Reuse the same dependency projection for the fact scan.
    let fact_width = relations[0].names.len();
    let projected = &projections[0];
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

#[cfg(test)]
mod stream_output_limit_tests {
    use super::*;
    use super::super::super::Number;

    #[test]
    fn streaming_inner_joins_can_emit_more_than_the_materialized_row_cap() {
        let fact = typed::Cell::Numeric(Number::Unsigned(7));
        let dimension = typed::Cell::Numeric(Number::Unsigned(7));
        let right_rows = vec![vec![dimension]];
        let mut hash = HashMap::new();
        hash.insert(row_key(&right_rows[0], &[0]).unwrap(), Bucket::One(0));
        let stages = [Stage {
            rows: right_rows,
            hash,
            left_keys: vec![0],
            any: false,
        }];
        let selected = [(0, "fact.id".to_owned(), None)];
        let spec = OutputSpec {
            selected: &selected,
            null_filter: None,
            offset: 0,
            limit: None,
        };
        let mut output = std::io::sink();
        let mut state = OutputState::default();
        let mut row = vec![fact];
        for _ in 0..131_100 {
            visit(0, &mut row, &stages, &spec, &mut state, &mut output).unwrap();
        }
        assert_eq!(state.seen, 131_100);
        assert_eq!(state.emitted, 131_100);
        assert_eq!(row.len(), 1);
    }

    #[test]
    fn streaming_inner_join_still_respects_explicit_limit() {
        let fact = typed::Cell::Numeric(Number::Unsigned(7));
        let dimension = typed::Cell::Numeric(Number::Unsigned(7));
        let right_rows = vec![vec![dimension]];
        let mut hash = HashMap::new();
        hash.insert(row_key(&right_rows[0], &[0]).unwrap(), Bucket::One(0));
        let stages = [Stage {
            rows: right_rows,
            hash,
            left_keys: vec![0],
            any: false,
        }];
        let selected = [(0, "fact.id".to_owned(), None)];
        let spec = OutputSpec {
            selected: &selected,
            null_filter: None,
            offset: 0,
            limit: Some(2),
        };
        let mut state = OutputState::default();
        let mut row = vec![fact];
        let mut output = std::io::sink();
        for _ in 0..100 {
            visit(0, &mut row, &stages, &spec, &mut state, &mut output).unwrap();
        }
        assert_eq!(state.emitted, 2);
    }
}

#[cfg(test)]
mod streaming_hash_budget_tests {
    use super::*;

    #[test]
    fn large_string_keys_are_charged_in_addition_to_dimension_rows() {
        let key = [typed::Key::Text(vec![b'x'; 2048])];
        assert!(hash_key_bytes(&key).unwrap() > 2048);
        let mut used = MAX_CHAIN_BYTES - 100;
        assert!(charge_hash_build(&mut used, hash_key_bytes(&key).unwrap()).is_err());
        assert_eq!(used, MAX_CHAIN_BYTES - 100);
    }

    #[test]
    fn hash_accounting_rejects_overflow_before_allocation() {
        let mut used = 0usize;
        assert!(charge_hash_build(&mut used, usize::MAX).is_err());
        assert_eq!(used, 0);
        charge_hash_build(&mut used, 128).unwrap();
        assert_eq!(used, 128);
    }
}
