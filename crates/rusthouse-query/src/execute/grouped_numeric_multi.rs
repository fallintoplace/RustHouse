//! Multiple grouped numeric aggregates over one typed scan.
use super::grouped_filter::bind_grouped_filters;
use super::typed::{self, Cell};
use super::{AggregateState, ExecutionError, Number};
use crate::{AggregateFunction, Select};
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::collections::{HashMap, HashSet, hash_map::Entry};
use std::hash::BuildHasher;
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 200_000;

/// Prepared presentation and input metadata for one grouped aggregate metric.
/// Resolved once per query instead of rebuilding it for every output header.
#[derive(Clone, Copy)]
struct GroupedMetricDescriptor<'a> {
    function: AggregateFunction,
    column: &'a str,
    alias: Option<&'a str>,
    filter: Option<&'a crate::GroupedFilter>,
}

impl GroupedMetricDescriptor<'_> {
    fn write_header<W: Write>(self, output: &mut W) -> Result<(), ExecutionError> {
        if let Some(alias) = self.alias {
            write!(output, "\t{alias}")?;
        } else if let Some(filter) = self.filter {
            write!(
                output,
                "\t{}If({}, {})",
                self.function.name(),
                self.column,
                filter.label()
            )?;
        } else {
            write!(output, "\t{}({})", self.function.name(), self.column)?;
        }
        Ok(())
    }
}

struct MetricState {
    numeric: AggregateState,
    text_value: Option<Vec<u8>>,
}
impl MetricState {
    fn new(function: AggregateFunction) -> Self {
        Self {
            numeric: AggregateState::new(function),
            text_value: None,
        }
    }
    fn sort_cell(&self) -> Cell {
        self.text_value.as_ref().map_or_else(
            || self.numeric.sort_cell(),
            |value| Cell::Text(value.clone()),
        )
    }
}
impl std::ops::Deref for MetricState {
    type Target = AggregateState;
    fn deref(&self) -> &Self::Target {
        &self.numeric
    }
}
impl std::ops::DerefMut for MetricState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.numeric
    }
}

enum TextGroupBucket {
    One(usize),
    Multiple(Vec<usize>),
}

fn find_text_group(
    groups: &[(Cell, Vec<MetricState>)],
    bucket: &TextGroupBucket,
    key: &[u8],
) -> Option<usize> {
    let matches = |index: usize| matches!(&groups[index].0, Cell::Text(existing) if existing.as_slice() == key);
    match bucket {
        TextGroupBucket::One(index) => matches(*index).then_some(*index),
        TextGroupBucket::Multiple(indices) => {
            indices.iter().find(|&&index| matches(index)).copied()
        }
    }
}

fn insert_text_group(bucket: &mut TextGroupBucket, index: usize) {
    match bucket {
        TextGroupBucket::One(first) => {
            let first = *first;
            *bucket = TextGroupBucket::Multiple(vec![first, index]);
        }
        TextGroupBucket::Multiple(indices) => indices.push(index),
    }
}

pub(super) struct DistinctValues {
    values: HashSet<(usize, usize, typed::Key)>,
    retained: usize,
}

impl DistinctValues {
    pub(super) fn new() -> Self {
        Self {
            values: HashSet::new(),
            retained: 0,
        }
    }

    pub(super) fn insert(
        &mut self,
        group: usize,
        metric: usize,
        key: typed::Key,
    ) -> Result<bool, ExecutionError> {
        let entry = (group, metric, key);
        if self.values.contains(&entry) {
            return Ok(false);
        }
        if self.values.len() >= 200_000 {
            return Err(ExecutionError::Invalid(
                "grouped COUNT(DISTINCT) supports at most 200000 distinct pairs".into(),
            ));
        }
        let dynamic = match &entry.2 {
            typed::Key::Text(value) => value.len(),
            typed::Key::Uuid(value) => value.len(),
            _ => 0,
        };
        let charge = std::mem::size_of_val(&entry)
            .saturating_mul(2)
            .saturating_add(dynamic);
        self.retained = self
            .retained
            .checked_add(charge)
            .ok_or_else(|| ExecutionError::Invalid("distinct state memory overflow".into()))?;
        if self.retained > 64 * 1024 * 1024 {
            return Err(ExecutionError::Invalid(
                "grouped distinct values exceed 64 MiB retained budget".into(),
            ));
        }
        self.values.insert(entry);
        Ok(true)
    }
}

fn group_limit_error() -> ExecutionError {
    ExecutionError::Invalid("GROUP BY supports at most 200000 groups".into())
}

fn numeric_column_error() -> ExecutionError {
    ExecutionError::Invalid("numeric aggregates require numeric columns".into())
}

// Conditional metrics only need zero/nonzero, not a widened Number per row.
fn numeric_nonzero_at(values: &NumericColumn, row: usize) -> bool {
    match values {
        NumericColumn::UInt8(values) => values[row] != 0,
        NumericColumn::UInt16(values) => values[row] != 0,
        NumericColumn::UInt32(values) => values[row] != 0,
        NumericColumn::UInt64(values) => values[row] != 0,
        NumericColumn::UInt128(values) => values[row] != 0,
        NumericColumn::Int8(values) => values[row] != 0,
        NumericColumn::Int16(values) => values[row] != 0,
        NumericColumn::Int32(values) => values[row] != 0,
        NumericColumn::Int64(values) => values[row] != 0,
        NumericColumn::Int128(values) => values[row] != 0,
        NumericColumn::Float32(values) => values[row] != 0.0,
        NumericColumn::Float64(values) => values[row] != 0.0,
    }
}

fn filter_enabled(column: &TypedColumn, row: usize) -> Result<bool, ExecutionError> {
    match column {
        TypedColumn::Numeric(values) => Ok(numeric_nonzero_at(values, row)),
        TypedColumn::NullableNumeric { values, nulls } => {
            Ok(nulls[row] == 0 && numeric_nonzero_at(values, row))
        }
        _ => Err(ExecutionError::Invalid(
            "conditional metric filter must be numeric".into(),
        )),
    }
}

const MAX_GROUP_STATE_BYTES: usize = 64 * 1024 * 1024;

fn reserve_median_value(retained: &mut usize) -> Result<(), ExecutionError> {
    let bytes = retained
        .checked_add(std::mem::size_of::<Number>())
        .ok_or_else(|| ExecutionError::Invalid("grouped median memory overflow".into()))?;
    if bytes > MAX_GROUP_STATE_BYTES {
        return Err(ExecutionError::Invalid(
            "grouped median values exceed 64 MiB retained budget".into(),
        ));
    }
    *retained = bytes;
    Ok(())
}

fn reserve_group(
    retained: &mut usize,
    metric_count: usize,
    variable_bytes: usize,
    collision_bytes: usize,
) -> Result<(), ExecutionError> {
    let fixed = std::mem::size_of::<(Cell, Vec<MetricState>)>()
        .checked_add(std::mem::size_of::<typed::Key>())
        .and_then(|v| {
            metric_count
                .checked_mul(std::mem::size_of::<MetricState>())
                .and_then(|states| v.checked_add(states))
        })
        .ok_or_else(|| ExecutionError::Invalid("group memory estimate overflow".into()))?;
    let next = variable_bytes
        .checked_add(collision_bytes)
        .and_then(|variable| fixed.checked_add(variable))
        .and_then(|additional| retained.checked_add(additional))
        .ok_or_else(|| ExecutionError::Invalid("group memory estimate overflow".into()))?;
    if next > MAX_GROUP_STATE_BYTES {
        return Err(ExecutionError::Invalid(
            "GROUP BY exceeds 64 MiB group-state budget".into(),
        ));
    }
    *retained = next;
    Ok(())
}

// Keep 8-bit groups in direct slots, but only allocate the large 16-bit
// table when observed cardinality makes the sparse lookup more expensive.
// Most GROUP BY scans with UInt16 keys visit a tiny part of that domain.
enum GroupSlots {
    // Each nullable numeric domain adds exactly one extra NULL key.
    Byte(Box<[usize; 257]>),
    WideSparse {
        map: HashMap<usize, usize>,
        probes: usize,
    },
    WideDense(Vec<usize>),
}

impl GroupSlots {
    fn for_type(kind: &str) -> Option<Self> {
        match typed::base_type(kind) {
            "UInt8" | "Int8" => Some(Self::Byte(Box::new([usize::MAX; 257]))),
            "UInt16" | "Int16" => Some(Self::WideSparse {
                map: HashMap::new(),
                probes: 0,
            }),
            _ => None,
        }
    }

    fn get(&mut self, key: usize) -> Option<usize> {
        if let Self::WideSparse { map, probes } = self {
            *probes += 1;
            // Even one frequently repeated UInt16 key is worth direct
            // indexing on long scans. Keep sparse maps for short scans.
            if *probes >= 8192 {
                let mut slots = vec![usize::MAX; 65_537];
                for (&value, &group) in map.iter() {
                    slots[value] = group;
                }
                *self = Self::WideDense(slots);
            }
        }
        match self {
            Self::Byte(slots) => (slots[key] != usize::MAX).then_some(slots[key]),
            Self::WideSparse { map, .. } => map.get(&key).copied(),
            Self::WideDense(slots) => (slots[key] != usize::MAX).then_some(slots[key]),
        }
    }

    fn insert(&mut self, key: usize, index: usize) {
        match self {
            Self::Byte(slots) => slots[key] = index,
            Self::WideDense(slots) => slots[key] = index,
            Self::WideSparse { map, .. } => {
                map.insert(key, index);
                if map.len() >= 2048 {
                    let mut slots = vec![usize::MAX; 65_537];
                    for (&value, &group) in map.iter() {
                        slots[value] = group;
                    }
                    *self = Self::WideDense(slots);
                }
            }
        }
    }
}

// Directly distinguish NULL from every valid signed/unsigned bit pattern.
// Nullable byte and short grouping keys never require a generic hashed Cell.
fn numeric_group_slot(column: &TypedColumn, row: usize) -> Option<usize> {
    let (values, is_null) = match column {
        TypedColumn::Numeric(values) => (values, false),
        TypedColumn::NullableNumeric { values, nulls } => (values, nulls[row] != 0),
        _ => return None,
    };
    match values {
        NumericColumn::UInt8(values) => Some(if is_null {
            256
        } else {
            usize::from(values[row])
        }),
        NumericColumn::Int8(values) => Some(if is_null {
            256
        } else {
            usize::from(values[row] as u8)
        }),
        NumericColumn::UInt16(values) => Some(if is_null {
            65_536
        } else {
            usize::from(values[row])
        }),
        NumericColumn::Int16(values) => Some(if is_null {
            65_536
        } else {
            usize::from(values[row] as u16)
        }),
        _ => None,
    }
}

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    column: &str,
    items: &[(AggregateFunction, String)],
    aliases: &[Option<String>],
    filters: &[Option<crate::GroupedFilter>],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata = inspect_query_source(part)?;
    // This index is also needed by predicate compilation below. Build it
    // before metric validation instead of scanning metadata for every field.
    let types = metadata
        .columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect::<HashMap<_, _>>();
    let mut names = vec![column.to_owned()];
    for (metric, (function, value_column)) in items.iter().enumerate() {
        if let Some(Some(expression)) = query.grouped_expressions.get(metric) {
            expression.referenced_columns(&mut names);
            continue;
        }
        if *function == AggregateFunction::Count && value_column == "*" {
            continue;
        }
        if !names.contains(value_column) {
            names.push(value_column.clone());
        }
        let selected = types.get(value_column).ok_or_else(|| {
            ExecutionError::Invalid(format!("unknown aggregate column: {value_column}"))
        })?;
        if !matches!(
            function,
            AggregateFunction::Count | AggregateFunction::CountDistinct
        ) && matches!(typed::base_type(selected), "String" | "UUID")
            && !(typed::base_type(selected) == "String"
                && matches!(function, AggregateFunction::Min | AggregateFunction::Max))
        {
            return Err(numeric_column_error());
        }
    }
    let text_metrics = items
        .iter()
        .enumerate()
        .map(|(index, (function, column))| {
            !query
                .grouped_expressions
                .get(index)
                .is_some_and(Option::is_some)
                && matches!(function, AggregateFunction::Min | AggregateFunction::Max)
                && types
                    .get(column)
                    .is_some_and(|kind| typed::base_type(kind) == "String")
        })
        .collect::<Vec<_>>();
    let descriptors = items
        .iter()
        .enumerate()
        .map(|(index, (function, column))| GroupedMetricDescriptor {
            function: *function,
            column,
            alias: aliases.get(index).and_then(Option::as_deref),
            filter: filters.get(index).and_then(Option::as_ref),
        })
        .collect::<Vec<_>>();
    for (metric, is_text) in text_metrics.iter().enumerate() {
        if *is_text
            && matches!(
                filters.get(metric),
                Some(Some(
                    crate::GroupedFilter::CaseElseZero { .. }
                        | crate::GroupedFilter::CaseElseZeroCombined { .. }
                        | crate::GroupedFilter::CaseExpression { .. }
                ))
            )
        {
            return Err(ExecutionError::Invalid(
                "text extrema with numeric CASE fallback are unsupported".into(),
            ));
        }
    }
    if query
        .having
        .as_ref()
        .and_then(|having| having.aggregate_index)
        .is_some_and(|index| text_metrics.get(index) == Some(&true))
    {
        return Err(ExecutionError::Invalid(
            "HAVING on text aggregates is unsupported".into(),
        ));
    }
    for filter in filters.iter().flatten() {
        if let crate::GroupedFilter::CaseExpression { condition, .. } = filter {
            condition.referenced_columns(&mut names);
            continue;
        }
        let name = filter.column().expect("column filter has a column");
        if !names.iter().any(|column| column == name) {
            names.push(name.to_owned());
        }
        let column_type = types
            .get(name)
            .ok_or_else(|| ExecutionError::Invalid(format!("unknown filter column: {name}")))?;
        if matches!(filter, crate::GroupedFilter::Truthy(_))
            && matches!(typed::base_type(column_type), "String" | "UUID")
        {
            return Err(ExecutionError::Invalid(
                "conditional metric filters must be numeric".into(),
            ));
        }
    }
    for predicate in query
        .predicates
        .iter()
        .chain(query.or_predicates.iter().flatten())
    {
        let name = predicate.column();
        if !names.iter().any(|column| column == name) {
            names.push(name.to_owned());
        }
        if let Some(other) = predicate.other_column() {
            if !names.iter().any(|column| column == other) {
                names.push(other.to_owned());
            }
        }
    }
    let indices = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<_, _>>();
    let group_type = types
        .get(column)
        .ok_or_else(|| ExecutionError::Invalid(format!("unknown group column: {column}")))?;
    let bound_expressions = query
        .grouped_expressions
        .iter()
        .map(|expression| {
            expression
                .as_ref()
                .map(|expr| super::multi_aggregate::BoundExpr::bind(expr, &indices))
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut conditions = vec![typed::compile(&query.predicates, &indices, &types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch, &indices, &types)?);
    }
    write!(output, "{column}")?;
    for metric in &descriptors {
        metric.write_header(output)?;
    }
    writeln!(output)?;
    if query.limit == Some(0) {
        return Ok(());
    }

    let group_index = indices[column];
    let item_indices = descriptors
        .iter()
        .enumerate()
        .map(|(metric, descriptor)| {
            if bound_expressions.get(metric).is_some_and(Option::is_some) {
                None
            } else {
                (descriptor.function != AggregateFunction::Count || descriptor.column != "*")
                    .then(|| indices[descriptor.column])
            }
        })
        .collect::<Vec<_>>();
    let filter_indices = bind_grouped_filters(filters, &indices, &types)?;
    let mut groups = Vec::<(Cell, Vec<MetricState>)>::new();
    let mut distinct_values = DistinctValues::new();
    let mut retained_group_bytes = 0usize;
    let mut retained_median_bytes = 0usize;
    // Small groups keep an inexpensive map until direct indexing pays off.
    let mut dense_slots = GroupSlots::for_type(group_type);
    let mut by_key = HashMap::<typed::Key, usize>::new();
    let mut by_text = HashMap::<u64, TextGroupBucket>::new();
    let text_hasher = std::collections::hash_map::RandomState::new();
    let mut by_uuid = HashMap::<[u8; 16], usize>::new();
    let mut reader = TypedQueryReader::open_with_metadata(part, &names, &metadata)?;
    let no_filters = query.predicates.is_empty() && query.or_predicates.is_empty();
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch, row, &conditions) {
                continue;
            }
            // Nullable String keys can use the same borrowed-byte lookup as String.
            // Only new groups need to own the decoded bytes.
            let index = if let Some(slots) = dense_slots.as_mut() {
                let key = numeric_group_slot(&batch[group_index], row).ok_or_else(|| {
                    ExecutionError::Invalid("unexpected dense GROUP BY type".into())
                })?;
                if let Some(index) = slots.get(key) {
                    index
                } else {
                    reserve_group(&mut retained_group_bytes, items.len(), 0, 0)?;
                    let index = groups.len();
                    groups.push((
                        typed::cell(&batch[group_index], row),
                        descriptors
                                .iter()
                                .map(|metric| MetricState::new(metric.function))
                            .collect(),
                    ));
                    slots.insert(key, index);
                    index
                }
            } else {
                let borrowed_text = match &batch[group_index] {
                    TypedColumn::Text(values) => values.get(row),
                    TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => {
                        values.get(row)
                    }
                    _ => None,
                };
                let borrowed_uuid = match &batch[group_index] {
                    TypedColumn::Uuid(values) => Some(*values[row].as_bytes()),
                    _ => None,
                };
                if let Some(key) = borrowed_uuid {
                    if let Some(&group) = by_uuid.get(&key) {
                        group
                    } else {
                        if groups.len() >= MAX_GROUPS {
                            return Err(group_limit_error());
                        }
                        reserve_group(&mut retained_group_bytes, items.len(), 0, 0)?;
                        let group = groups.len();
                        by_uuid.insert(key, group);
                        groups.push((
                            typed::cell(&batch[group_index], row),
                            descriptors
                                .iter()
                                .map(|metric| MetricState::new(metric.function))
                                .collect(),
                        ));
                        group
                    }
                } else if let Some(key) = borrowed_text {
                    let hash = text_hasher.hash_one(key);
                    match by_text.entry(hash) {
                        Entry::Vacant(entry) => {
                            if groups.len() >= MAX_GROUPS {
                                return Err(group_limit_error());
                            }
                            reserve_group(&mut retained_group_bytes, items.len(), key.len(), 0)?;
                            let group = groups.len();
                            entry.insert(TextGroupBucket::One(group));
                            groups.push((
                                Cell::Text(key.to_vec()),
                                descriptors
                                .iter()
                                .map(|metric| MetricState::new(metric.function))
                                    .collect(),
                            ));
                            group
                        }
                        Entry::Occupied(mut entry) => {
                            if let Some(group) = find_text_group(&groups, entry.get(), key) {
                                group
                            } else {
                                if groups.len() >= MAX_GROUPS {
                                    return Err(group_limit_error());
                                }
                                reserve_group(
                                    &mut retained_group_bytes,
                                    items.len(),
                                    key.len(),
                                    2 * std::mem::size_of::<usize>(),
                                )?;
                                let group = groups.len();
                                insert_text_group(entry.get_mut(), group);
                                groups.push((
                                    Cell::Text(key.to_vec()),
                                    descriptors
                                .iter()
                                .map(|metric| MetricState::new(metric.function))
                                        .collect(),
                                ));
                                group
                            }
                        }
                    }
                } else {
                    let key = typed::cell(&batch[group_index], row);
                    match by_key.entry(key.key()) {
                        Entry::Occupied(entry) => *entry.get(),
                        Entry::Vacant(entry) => {
                            if groups.len() >= MAX_GROUPS {
                                return Err(group_limit_error());
                            }
                            reserve_group(&mut retained_group_bytes, items.len(), 0, 0)?;
                            entry.insert(groups.len());
                            groups.push((
                                key,
                                descriptors
                                .iter()
                                .map(|metric| MetricState::new(metric.function))
                                    .collect(),
                            ));
                            groups.len() - 1
                        }
                    }
                }
            };
            for (metric, ((function, _), (column_index, state))) in items
                .iter()
                .zip(item_indices.iter().zip(groups[index].1.iter_mut()))
                .enumerate()
            {
                if let Some(filter) = filter_indices.get(metric).and_then(Option::as_ref) {
                    if !filter.passes(&batch, row, filter_enabled)? {
                        // CASE ... ELSE 0 contributes a zero for unmatched rows.
                        // Plain SUMIF and CASE without ELSE still skip the row.
                        if *function != AggregateFunction::Count
                            && matches!(
                                filters.get(metric),
                                Some(Some(crate::GroupedFilter::CaseElseZero { .. }))
                            )
                        {
                            if *function == AggregateFunction::Sum {
                                state.update_sum(Number::Unsigned(0))?;
                            } else {
                                state.update(Number::Unsigned(0))?;
                            }
                        }
                        continue;
                    }
                }
                if text_metrics[metric] {
                    let column_index = column_index.expect("text metric has a column");
                    let candidate = match &batch[column_index] {
                        TypedColumn::Text(values) => values.get(row),
                        TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => {
                            values.get(row)
                        }
                        TypedColumn::NullableText { .. } => None,
                        _ => return Err(numeric_column_error()),
                    };
                    if let Some(candidate) = candidate {
                        let replace = state.text_value.as_deref().is_none_or(|current| {
                            if *function == AggregateFunction::Min {
                                candidate < current
                            } else {
                                candidate > current
                            }
                        });
                        if replace {
                            let old = state.text_value.as_ref().map_or(0, Vec::len);
                            let new_total = retained_group_bytes
                                .checked_sub(old)
                                .and_then(|size| size.checked_add(candidate.len()))
                                .ok_or_else(|| {
                                    ExecutionError::Invalid("group memory estimate overflow".into())
                                })?;
                            if new_total > MAX_GROUP_STATE_BYTES {
                                return Err(ExecutionError::Invalid(
                                    "GROUP BY exceeds 64 MiB group-state budget".into(),
                                ));
                            }
                            state.text_value = Some(candidate.to_vec());
                            retained_group_bytes = new_total;
                        }
                    }
                    continue;
                }
                if *function == AggregateFunction::CountDistinct {
                    let column_index = column_index.expect("distinct metric has a column");
                    if !typed::is_null_at(&batch[column_index], row)
                        && distinct_values.insert(
                            index,
                            metric,
                            typed::key_at(&batch[column_index], row),
                        )?
                    {
                        state.increment_count()?;
                    }
                    continue;
                }
                if let Some(Some(expression)) = bound_expressions.get(metric) {
                    if let Some(value) =
                        super::multi_aggregate::eval_number(expression, &batch, row)?
                    {
                        if *function == AggregateFunction::Median {
                            reserve_median_value(&mut retained_median_bytes)?;
                        }
                        if *function == AggregateFunction::Sum {
                            state.update_sum(value)?;
                        } else {
                            state.update(value)?;
                        }
                    }
                    continue;
                }
                if *function == AggregateFunction::Count {
                    if let Some(column_index) = *column_index {
                        if !typed::is_null_at(&batch[column_index], row) {
                            state.increment_count()?;
                        }
                    } else {
                        state.increment_count()?;
                    }
                    continue;
                }
                let column_index = column_index.expect("non-COUNT(*) metric has a column");
                if *function == AggregateFunction::Median
                    && !typed::is_null_at(&batch[column_index], row)
                {
                    reserve_median_value(&mut retained_median_bytes)?;
                }
                // Handle native numeric batches without creating a Cell
                // for each aggregate value and every matching input row.
                match &batch[column_index] {
                    TypedColumn::Numeric(values) => {
                        let value = super::number_at(values, row);
                        if *function == AggregateFunction::Sum {
                            state.update_sum(value)?;
                        } else {
                            state.update(value)?;
                        }
                    }
                    TypedColumn::NullableNumeric { values, nulls } => {
                        if nulls[row] == 0 {
                            let value = super::number_at(values, row);
                            if *function == AggregateFunction::Sum {
                                state.update_sum(value)?;
                            } else {
                                state.update(value)?;
                            }
                        }
                    }
                    column => match typed::cell(column, row) {
                        Cell::Numeric(value) => state.update(value)?,
                        Cell::Null => {}
                        Cell::Text(_) | Cell::Uuid(_) => {
                            return Err(numeric_column_error());
                        }
                    },
                }
            }
        }
    }
    if let Some(having) = &query.having {
        let index = having
            .aggregate_index
            .ok_or_else(|| ExecutionError::Invalid("HAVING metric index is required".into()))?;
        groups.retain(|(_, states)| {
            states
                .get(index)
                .is_some_and(|state| state.passes_having(having))
        });
    }
    let mut bounded_order = None;
    if query.order_by.is_some() {
        let terms = query
            .order_terms()
            .map(|order| {
                let metric = order
                    .column
                    .strip_prefix("#aggregate")
                    .and_then(|number| number.parse::<usize>().ok());
                (metric, order.descending, order.nulls_first)
            })
            .collect::<Vec<_>>();
        let compare = |left: &(Cell, Vec<MetricState>), right: &(Cell, Vec<MetricState>)| {
            for &(metric, descending, nulls_first) in &terms {
                let ordering = if let Some(index) = metric {
                    typed::compare_order_cells(
                        &left.1[index].sort_cell(),
                        &right.1[index].sort_cell(),
                        descending,
                        nulls_first,
                    )
                } else {
                    typed::compare_order_cells(&left.0, &right.0, descending, nulls_first)
                };
                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
                }
            }
            std::cmp::Ordering::Equal
        };
        let end = query
            .limit
            .and_then(|limit| query.offset.unwrap_or(0).checked_add(limit));
        let cached_medians = terms
            .iter()
            .filter_map(|(metric, _, _)| *metric)
            .filter(|&metric| items[metric].0 == AggregateFunction::Median)
            .map(|metric| {
                (
                    metric,
                    groups
                        .iter()
                        .map(|(_, states)| states[metric].sort_cell())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<HashMap<_, _>>();
        if !cached_medians.is_empty() {
            // Avoid cloning a median's entire input vector in each
            // comparison, including multi-term ORDER BY and paged top-K.
            let mut indices = (0..groups.len()).collect::<Vec<_>>();
            let compare_cached = |left: &usize, right: &usize| {
                for &(metric, descending, nulls_first) in &terms {
                    let order = if let Some(index) = metric {
                        if let Some(keys) = cached_medians.get(&index) {
                            typed::compare_order_cells(
                                &keys[*left],
                                &keys[*right],
                                descending,
                                nulls_first,
                            )
                        } else {
                            let left_cell = groups[*left].1[index].sort_cell();
                            let right_cell = groups[*right].1[index].sort_cell();
                            typed::compare_order_cells(
                                &left_cell,
                                &right_cell,
                                descending,
                                nulls_first,
                            )
                        }
                    } else {
                        typed::compare_order_cells(
                            &groups[*left].0,
                            &groups[*right].0,
                            descending,
                            nulls_first,
                        )
                    };
                    if order != std::cmp::Ordering::Equal {
                        return order;
                    }
                }
                left.cmp(right)
            };
            if let Some(end) = end.filter(|&end| end < indices.len()) {
                indices.select_nth_unstable_by(end, &compare_cached);
                indices.truncate(end);
            }
            indices.sort_unstable_by(compare_cached);
            bounded_order = Some(indices);
        } else if let Some(end) = end.filter(|&end| end < groups.len()) {
            let mut indices = (0..groups.len()).collect::<Vec<_>>();
            let compare_index = |left: &usize, right: &usize| {
                compare(&groups[*left], &groups[*right]).then_with(|| left.cmp(right))
            };
            indices.select_nth_unstable_by(end, &compare_index);
            indices.truncate(end);
            indices.sort_unstable_by(compare_index);
            bounded_order = Some(indices);
        } else {
            groups.sort_by(compare);
        }
    }
    let write_group =
        |key: &Cell, states: &[MetricState], output: &mut W| -> Result<(), ExecutionError> {
            typed::write_typed_value(output, key, group_type)?;
            for (metric, (state, (function, column))) in states.iter().zip(items).enumerate() {
                if text_metrics[metric] {
                    write!(output, "\t")?;
                    typed::write_value(output, &state.sort_cell())?;
                    continue;
                }
                let value = if matches!(function, AggregateFunction::Min | AggregateFunction::Max) {
                    typed::format_temporal_aggregate(state.render(), &types[column])
                } else {
                    state.render()
                };
                write!(output, "\t{value}")?;
            }
            writeln!(output)?;
            Ok(())
        };
    if let Some(indices) = bounded_order {
        for index in indices.into_iter().skip(query.offset.unwrap_or(0)) {
            write_group(&groups[index].0, &groups[index].1, output)?;
        }
    } else {
        for (key, states) in groups
            .into_iter()
            .skip(query.offset.unwrap_or(0))
            .take(query.limit.unwrap_or(usize::MAX))
        {
            write_group(&key, &states, output)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod grouped_metric_descriptor_tests {
    use super::*;

    #[test]
    fn prepared_descriptor_preserves_default_and_alias_headers() {
        let plain = GroupedMetricDescriptor {
            function: AggregateFunction::Sum,
            column: "price",
            alias: None,
            filter: None,
        };
        let alias = GroupedMetricDescriptor { alias: Some("total"), ..plain };
        let mut output = Vec::new();
        plain.write_header(&mut output).unwrap();
        alias.write_header(&mut output).unwrap();
        assert_eq!(output, b"\tsum(price)\ttotal");
    }
}

#[cfg(test)]
mod group_state_budget_tests {
    use super::*;

    #[test]
    fn bounded_multi_metric_group_memory_cannot_wrap_or_overrun() {
        let mut used = 0usize;
        reserve_group(&mut used, 4, 128, 0).unwrap();
        let first = used;
        reserve_group(&mut used, 4, 128, 0).unwrap();
        assert_eq!(used, first * 2);
        assert!(reserve_group(&mut used, usize::MAX, 0, 0).is_err());
        assert_eq!(used, first * 2);
        assert!(reserve_group(&mut used, 1, MAX_GROUP_STATE_BYTES, 0).is_err());
    }

    #[test]
    fn text_group_memory_counts_one_key_copy_and_collision_slots() {
        let mut fixed = 0;
        reserve_group(&mut fixed, 1, 0, 0).unwrap();

        let key_bytes = 128;
        let mut used = 0;
        reserve_group(&mut used, 1, key_bytes, 0).unwrap();
        assert_eq!(used, fixed + key_bytes);

        let collision_bytes = 2 * std::mem::size_of::<usize>();
        reserve_group(&mut used, 1, key_bytes, collision_bytes).unwrap();
        assert_eq!(used, 2 * (fixed + key_bytes) + collision_bytes);
    }
}

#[cfg(test)]
mod text_group_lookup_tests {
    use super::*;

    #[test]
    fn text_group_lookup_checks_values_after_hash_collisions() {
        let mut groups = vec![(Cell::Text(b"alpha".to_vec()), Vec::new())];
        let mut bucket = TextGroupBucket::One(0);
        assert_eq!(find_text_group(&groups, &bucket, b"alpha"), Some(0));
        assert_eq!(find_text_group(&groups, &bucket, b"missing"), None);

        groups.push((Cell::Text(b"beta".to_vec()), Vec::new()));
        insert_text_group(&mut bucket, 1);
        assert_eq!(find_text_group(&groups, &bucket, b"alpha"), Some(0));
        assert_eq!(find_text_group(&groups, &bucket, b"beta"), Some(1));
        assert_eq!(find_text_group(&groups, &bucket, b"gamma"), None);
    }
}

#[cfg(test)]
mod borrowed_metric_filter_tests {
    use super::*;
    #[test]
    fn handles_all_numeric_filter_types_without_materializing_cells() {
        let unsigned = TypedColumn::Numeric(NumericColumn::UInt8(vec![0, 1]));
        assert!(!filter_enabled(&unsigned, 0).unwrap());
        assert!(filter_enabled(&unsigned, 1).unwrap());
        let signed = TypedColumn::Numeric(NumericColumn::Int64(vec![-4, 0]));
        assert!(filter_enabled(&signed, 0).unwrap());
        assert!(!filter_enabled(&signed, 1).unwrap());
        let nullable = TypedColumn::NullableNumeric {
            values: NumericColumn::UInt64(vec![99, 0]),
            nulls: vec![1, 0],
        };
        assert!(!filter_enabled(&nullable, 0).unwrap());
        assert!(!filter_enabled(&nullable, 1).unwrap());
        let floats = TypedColumn::Numeric(NumericColumn::Float64(vec![0.0, 2.5]));
        assert!(!filter_enabled(&floats, 0).unwrap());
        assert!(filter_enabled(&floats, 1).unwrap());
    }
}

#[cfg(test)]
mod grouped_multi_median_budget_tests {
    use super::*;

    #[test]
    fn bounds_median_storage_across_metrics_and_groups() {
        let mut retained = 0;
        reserve_median_value(&mut retained).unwrap();
        let one = retained;
        reserve_median_value(&mut retained).unwrap();
        assert_eq!(retained, one * 2);
        retained = MAX_GROUP_STATE_BYTES - 1;
        assert!(reserve_median_value(&mut retained).is_err());
        assert_eq!(retained, MAX_GROUP_STATE_BYTES - 1);
    }
}

#[cfg(test)]
mod cached_multimetric_median_sort_tests {
    use super::*;
    #[test]
    fn borrowed_cached_medians_keep_same_sort_order() {
        let values = [3u128, 9, 1, 7];
        let keys = values.map(|value| {
            let mut median = AggregateState::new(AggregateFunction::Median);
            median.update(Number::Unsigned(value)).unwrap();
            median.sort_cell()
        });
        let mut positions = (0..keys.len()).collect::<Vec<_>>();
        positions
            .sort_unstable_by(|&a, &b| typed::compare_order_cells(&keys[a], &keys[b], false, None));
        assert_eq!(positions, [2, 0, 3, 1]);
    }

    #[test]
    fn cached_medians_preserve_multi_metric_order() {
        let groups = [(10u128, 4u128), (4, 2), (7, 3)].map(|(a, b)| {
            let mut state = AggregateState::new(AggregateFunction::Median);
            state.update(Number::Unsigned(a)).unwrap();
            state.update(Number::Unsigned(b)).unwrap();
            state
        });
        let cached = groups
            .iter()
            .map(AggregateState::sort_cell)
            .collect::<Vec<_>>();
        let mut order = (0..groups.len()).collect::<Vec<_>>();
        order.sort_by(|&left, &right| {
            typed::compare_order_cells(&cached[left], &cached[right], false, None)
        });
        assert_eq!(order, [1, 2, 0]);
    }
}

#[cfg(test)]
mod lazy_u16_multi_aggregate_index_tests {
    use super::*;

    #[test]
    fn byte_domain_keeps_compact_direct_lookup() {
        for kind in ["UInt8", "Int8"] {
            let mut slots = GroupSlots::for_type(kind).unwrap();
            assert!(matches!(slots, GroupSlots::Byte(_)));
            for key in 0..256 {
                assert_eq!(slots.get(key), None);
                slots.insert(key, 255 - key);
                assert_eq!(slots.get(key), Some(255 - key));
            }
        }
    }

    #[test]
    fn sparse_u16_groups_promote_and_preserve_signed_keys_and_indexes() {
        for kind in ["UInt16", "Int16"] {
            let mut slots = GroupSlots::for_type(kind).unwrap();
            assert!(matches!(slots, GroupSlots::WideSparse { .. }));
            for key in 0..2047 {
                assert_eq!(slots.get(key), None);
                slots.insert(key, key + 17);
            }
            assert!(matches!(slots, GroupSlots::WideSparse { .. }));
            slots.insert(65_535, 99_999);
            assert!(matches!(slots, GroupSlots::WideDense(_)));
            assert_eq!(slots.get(65_535), Some(99_999));
            assert_eq!(slots.get(2047), None);
            for key in 0..2047 {
                assert_eq!(slots.get(key), Some(key + 17));
            }
            for key in 2047..65_535 {
                slots.insert(key, key + 17);
            }
            for key in 0..65_535 {
                assert_eq!(slots.get(key), Some(key + 17));
            }
        }
    }
}

#[cfg(test)]
mod nullable_numeric_group_slots_tests {
    use super::*;

    #[test]
    fn byte_and_short_nulls_do_not_alias_numeric_extremes() {
        let bytes = TypedColumn::NullableNumeric {
            values: NumericColumn::Int8(vec![-1, 0, 127, -1]),
            nulls: vec![0, 0, 0, 1],
        };
        let shorts = TypedColumn::NullableNumeric {
            values: NumericColumn::UInt16(vec![0, 65_535, 0]),
            nulls: vec![0, 0, 1],
        };
        assert_eq!(
            (0..4)
                .map(|i| numeric_group_slot(&bytes, i))
                .collect::<Vec<_>>(),
            vec![Some(255), Some(0), Some(127), Some(256)]
        );
        assert_eq!(
            (0..3)
                .map(|i| numeric_group_slot(&shorts, i))
                .collect::<Vec<_>>(),
            vec![Some(0), Some(65_535), Some(65_536)]
        );
    }

    #[test]
    fn wide_sparse_to_dense_promotion_keeps_the_null_slot() {
        let mut slots = GroupSlots::for_type("Nullable(Int16)").unwrap();
        slots.insert(65_536, 7);
        for key in 0..2048 {
            slots.insert(key, key + 10);
        }
        assert!(matches!(slots, GroupSlots::WideDense(_)));
        assert_eq!(slots.get(65_536), Some(7));
        assert_eq!(slots.get(0), Some(10));
        assert_eq!(slots.get(2047), Some(2057));
        assert_eq!(slots.get(65_535), None);
    }
}

#[cfg(test)]
mod native_conditional_filter_tests {
    use super::*;

    #[test]
    fn native_zero_probes_keep_nullable_and_nan_semantics() {
        let columns = [
            TypedColumn::Numeric(NumericColumn::UInt8(vec![0, 1, 255])),
            TypedColumn::Numeric(NumericColumn::Int64(vec![0, -1, i64::MAX])),
            TypedColumn::Numeric(NumericColumn::Float32(vec![0.0, -0.0, f32::NAN])),
            TypedColumn::Numeric(NumericColumn::Float64(vec![0.0, -0.0, f64::INFINITY])),
            TypedColumn::NullableNumeric {
                values: NumericColumn::UInt8(vec![1, 0, 1]),
                nulls: vec![1, 0, 0],
            },
        ];
        let expected = [
            [false, true, true],
            [false, true, true],
            [false, false, true],
            [false, false, true],
            [false, false, true],
        ];
        for (column, expected) in columns.iter().zip(expected) {
            for (row, enabled) in expected.into_iter().enumerate() {
                assert_eq!(filter_enabled(column, row).unwrap(), enabled);
            }
        }
        assert!(
            filter_enabled(
                &TypedColumn::Numeric(NumericColumn::UInt128(vec![u128::MAX])),
                0
            )
            .unwrap()
        );
    }
}

#[cfg(test)]
mod hot_short_group_promotion_tests {
    use super::*;

    #[test]
    fn repeated_short_key_probes_promote_without_changing_group_indexes() {
        let mut slots = GroupSlots::for_type("Nullable(UInt16)").unwrap();
        slots.insert(65_536, 0);
        slots.insert(42, 1);
        for _ in 0..8191 {
            assert_eq!(slots.get(42), Some(1));
        }
        assert!(matches!(slots, GroupSlots::WideSparse { .. }));
        assert_eq!(slots.get(42), Some(1));
        assert!(matches!(slots, GroupSlots::WideDense(_)));
        assert_eq!(slots.get(65_536), Some(0));
        assert_eq!(slots.get(43), None);
        slots.insert(43, 2);
        assert_eq!(slots.get(43), Some(2));
    }
}
