//! Build and apply bounded correlated EXISTS lookups.
use super::*;
use crate::{
    JoinExistsFilter, JoinExistsKey, JoinExistsLookup, JoinExistsLookupPair, RowPredicate,
};
use rusthouse_storage::{TypedQueryReader, inspect_query_source};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

const MAX_EXISTS_DATE_KEYS: usize = 200_000;

pub(super) struct BoundExists {
    pub(super) equality: usize,
    pub(super) inequality: Option<usize>,
    pub(super) negated: bool,
    lookup: BoundLookup,
}

enum BoundLookup {
    Single(JoinExistsLookup),
    Pair(Arc<JoinExistsLookupPair>, usize),
}

pub(super) fn integer(kind: &str) -> bool {
    matches!(
        typed::base_type(kind),
        "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UInt128"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "Int128"
    )
}

pub(super) fn key(value: &typed::Cell) -> Result<Option<JoinExistsKey>, ExecutionError> {
    match value {
        typed::Cell::Null => Ok(None),
        typed::Cell::Numeric(super::super::Number::Unsigned(value)) => {
            Ok(Some(JoinExistsKey::Unsigned(*value)))
        }
        typed::Cell::Numeric(super::super::Number::Signed(value)) => {
            Ok(Some(JoinExistsKey::Signed(*value)))
        }
        _ => Err(invalid("JOIN EXISTS requires integer keys")),
    }
}

fn lookup_for(
    relation: &[Relation],
    equality: usize,
    inequality: Option<usize>,
) -> JoinExistsLookup {
    let equality_type = typed::base_type(column_type(relation, equality));
    let inequality_type = inequality.map(|index| typed::base_type(column_type(relation, index)));
    let unsigned64 = |kind: &str| matches!(kind, "UInt8" | "UInt16" | "UInt32" | "UInt64");
    let signed64 = |kind: &str| matches!(kind, "Int8" | "Int16" | "Int32" | "Int64");
    if unsigned64(equality_type) && inequality_type.is_none_or(unsigned64) {
        JoinExistsLookup::new_compact64(inequality.is_some(), false)
    } else if signed64(equality_type) && inequality_type.is_none_or(signed64) {
        JoinExistsLookup::new_compact64(inequality.is_some(), true)
    } else {
        JoinExistsLookup::new(inequality.is_some())
    }
}

impl BoundExists {
    pub(super) fn passes(&self, row: &[typed::Cell]) -> Result<bool, ExecutionError> {
        let equal = key(&row[self.equality])?;
        let unequal = self
            .inequality
            .map(|index| key(&row[index]))
            .transpose()?
            .flatten();
        let exists = match &self.lookup {
            BoundLookup::Single(lookup) => lookup.exists(equal, unequal),
            BoundLookup::Pair(lookup, slot) => lookup.exists(*slot, equal, unequal),
        };
        Ok(exists != self.negated)
    }
}

pub(super) fn passes_expression(
    expression: &[Vec<usize>],
    filters: &[BoundExists],
    prechecked: &[usize],
    row: &[typed::Cell],
) -> Result<bool, ExecutionError> {
    if expression.is_empty() {
        for (index, filter) in filters.iter().enumerate() {
            if prechecked.contains(&index) {
                continue;
            }
            if !filter.passes(row)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    for group in expression {
        let mut passes = true;
        for &index in group {
            let filter = filters
                .get(index)
                .ok_or_else(|| invalid("JOIN EXISTS expression has an unknown filter"))?;
            if prechecked.contains(&index) {
                continue;
            }
            if !filter.passes(row)? {
                passes = false;
                break;
            }
        }
        if passes {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn fact_prefilters(
    filters: &[BoundExists],
    expression: &[Vec<usize>],
    fact_width: usize,
) -> Result<Vec<usize>, ExecutionError> {
    if expression
        .iter()
        .flatten()
        .any(|&index| index >= filters.len())
    {
        return Err(invalid("JOIN EXISTS expression has an unknown filter"));
    }
    Ok(filters
        .iter()
        .enumerate()
        .filter_map(|(index, filter)| {
            // A filter can move before the joins only if every OR branch requires it.
            let mandatory =
                expression.is_empty() || expression.iter().all(|group| group.contains(&index));
            let fact_keys = filter.equality < fact_width
                && filter.inequality.is_none_or(|key| key < fact_width);
            (mandatory && fact_keys).then_some(index)
        })
        .collect())
}

fn same_inner_column(left: &crate::JoinColumn, right: &crate::JoinColumn) -> bool {
    left.name.eq_ignore_ascii_case(&right.name)
}

fn can_share_pair(
    first: &JoinExistsFilter,
    second: &JoinExistsFilter,
    first_path: &Path,
    second_path: &Path,
) -> bool {
    if first.date_filter.is_some()
        || second.date_filter.is_some()
        || !first.table.eq_ignore_ascii_case(&second.table)
        || first_path != second_path
        || !same_inner_column(&first.equality.0, &second.equality.0)
    {
        return false;
    }
    match (&first.inequality, &second.inequality) {
        (Some((first, _)), Some((second, _))) => same_inner_column(first, second),
        _ => true,
    }
}

struct SharedInnerFilter {
    equality: (usize, usize),
    inequality: Option<(usize, usize)>,
    comparisons: Vec<(usize, crate::Operator, usize)>,
}

fn bind_shared_inner_filter(
    filter: &JoinExistsFilter,
    inner: &[Relation],
    outer: &[Relation],
) -> Result<SharedInnerFilter, ExecutionError> {
    let inner_index = |column: &crate::JoinColumn| {
        if column.relation != filter.alias {
            return Err(invalid("unknown JOIN EXISTS inner qualifier"));
        }
        column_index(inner, column)
    };
    let pair =
        |(left, right): &(crate::JoinColumn, crate::JoinColumn)| -> Result<_, ExecutionError> {
            let a = inner_index(left)?;
            let b = column_index(outer, right)?;
            let left_type = typed::base_type(column_type(inner, a));
            let right_type = typed::base_type(column_type(outer, b));
            if left_type != right_type || !integer(left_type) {
                return Err(invalid("JOIN EXISTS requires matching integer key types"));
            }
            Ok((a, b))
        };
    let equality = pair(&filter.equality)?;
    let inequality = filter.inequality.as_ref().map(pair).transpose()?;
    let mut comparisons = Vec::with_capacity(filter.inner_comparisons.len());
    for (left, operator, right) in &filter.inner_comparisons {
        let a = inner_index(left)?;
        let b = inner_index(right)?;
        let left_type = typed::base_type(column_type(inner, a));
        let right_type = typed::base_type(column_type(inner, b));
        if left_type != right_type
            || !matches!(
                operator,
                crate::Operator::Eq
                    | crate::Operator::Ne
                    | crate::Operator::Lt
                    | crate::Operator::Le
                    | crate::Operator::Gt
                    | crate::Operator::Ge
            )
        {
            return Err(invalid(
                "JOIN EXISTS inner comparison requires matching types",
            ));
        }
        comparisons.push((a, *operator, b));
    }
    Ok(SharedInnerFilter {
        equality,
        inequality,
        comparisons,
    })
}

fn shared_lookup_for(inner: &[Relation], plans: &[SharedInnerFilter; 2]) -> JoinExistsLookupPair {
    let equality_type = typed::base_type(column_type(inner, plans[0].equality.0));
    let inequality_type = plans.iter().find_map(|plan| {
        plan.inequality
            .map(|(index, _)| typed::base_type(column_type(inner, index)))
    });
    let unsigned64 = |kind: &str| matches!(kind, "UInt8" | "UInt16" | "UInt32" | "UInt64");
    let signed64 = |kind: &str| matches!(kind, "Int8" | "Int16" | "Int32" | "Int64");
    let inequalities = [plans[0].inequality.is_some(), plans[1].inequality.is_some()];
    if unsigned64(equality_type) && inequality_type.is_none_or(unsigned64) {
        JoinExistsLookupPair::new_compact64(inequalities, false)
    } else if signed64(equality_type) && inequality_type.is_none_or(signed64) {
        JoinExistsLookupPair::new_compact64(inequalities, true)
    } else {
        JoinExistsLookupPair::new(inequalities)
    }
}

fn bind_shared_pair(
    filters: [&JoinExistsFilter; 2],
    path: &Path,
    outer: &[Relation],
) -> Result<[BoundExists; 2], ExecutionError> {
    let metadata = inspect_query_source(path)?;
    let names = metadata
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let types = metadata
        .columns
        .iter()
        .map(|column| column.data_type.clone())
        .collect::<Vec<_>>();
    let make_inner = |alias: &str| Relation {
        alias: alias.to_owned(),
        names: names.clone(),
        types: types.clone(),
        physical_columns: names.len(),
        virtual_columns: Vec::new(),
        start: 0,
        rows: metadata.rows,
    };
    let inner = [make_inner(&filters[0].alias), make_inner(&filters[1].alias)];
    let plans = [
        bind_shared_inner_filter(filters[0], std::slice::from_ref(&inner[0]), outer)?,
        bind_shared_inner_filter(filters[1], std::slice::from_ref(&inner[1]), outer)?,
    ];
    if plans[0].equality.0 != plans[1].equality.0
        || plans[0]
            .inequality
            .zip(plans[1].inequality)
            .is_some_and(|(first, second)| first.0 != second.0)
    {
        return Err(invalid("shared JOIN EXISTS keys do not match"));
    }

    let common_inequality = plans[0]
        .inequality
        .or(plans[1].inequality)
        .map(|(index, _)| index);
    let mut projected = vec![plans[0].equality.0];
    projected.extend(common_inequality);
    for plan in &plans {
        for (left, _, right) in &plan.comparisons {
            projected.push(*left);
            projected.push(*right);
        }
    }
    projected.sort_unstable();
    projected.dedup();
    let position = |index: usize| projected.binary_search(&index).expect("projected column");
    let fields = projected
        .iter()
        .map(|&index| names[index].clone())
        .collect::<Vec<_>>();
    let mut reader = TypedQueryReader::open(path, &fields)?;
    let mut lookup = shared_lookup_for(std::slice::from_ref(&inner[0]), &plans);
    let equality_position = position(plans[0].equality.0);
    let inequality_position = common_inequality.map(position);
    let comparison_positions = plans
        .iter()
        .map(|plan| {
            plan.comparisons
                .iter()
                .map(|(left, operator, right)| (position(*left), *operator, position(*right)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut rows = 0u64;
    while let Some(batch) = reader.next_batch()? {
        for row in 0..batch[0].len() {
            rows = rows
                .checked_add(1)
                .ok_or_else(|| invalid("JOIN EXISTS row count overflow"))?;
            let equality = key(&typed::cell(&batch[equality_position], row))?;
            let inequality = inequality_position
                .map(|index| key(&typed::cell(&batch[index], row)))
                .transpose()?
                .flatten();
            for slot in 0..2 {
                let matches = comparison_positions[slot].iter().all(|(left, op, right)| {
                    typed::matches_comparison(
                        &typed::cell(&batch[*left], row),
                        &typed::cell(&batch[*right], row),
                        *op,
                    )
                });
                if matches {
                    lookup.insert(slot, equality, inequality)?;
                }
            }
        }
    }
    if rows != metadata.rows {
        return Err(invalid("JOIN EXISTS input row count changed during scan"));
    }
    let lookup = Arc::new(lookup);
    Ok(std::array::from_fn(|slot| BoundExists {
        equality: plans[slot].equality.1,
        inequality: plans[slot].inequality.map(|(_, outer)| outer),
        negated: filters[slot].negated,
        lookup: BoundLookup::Pair(Arc::clone(&lookup), slot),
    }))
}

pub(super) fn bind(
    filters: &[JoinExistsFilter],
    paths: &[PathBuf],
    outer: &[Relation],
) -> Result<Vec<BoundExists>, ExecutionError> {
    let expected_paths = filters
        .iter()
        .map(|filter| 1 + usize::from(filter.date_filter.is_some()))
        .sum::<usize>();
    if filters.is_empty() || filters.len() > 8 || paths.len() != expected_paths {
        return Err(invalid("JOIN EXISTS requires paths for each inner source"));
    }
    let mut result = Vec::with_capacity(filters.len());
    let mut filters = filters.iter().peekable();
    let mut paths = paths.iter().peekable();
    while let Some(filter) = filters.next() {
        if outer.iter().any(|relation| relation.alias == filter.alias) {
            return Err(invalid("JOIN EXISTS inner alias shadows an outer alias"));
        }
        let path = paths
            .next()
            .ok_or_else(|| invalid("JOIN EXISTS is missing its fact table path"))?;
        if let (Some(next_filter), Some(next_path)) = (filters.peek(), paths.peek()) {
            if can_share_pair(filter, next_filter, path, next_path) {
                if outer
                    .iter()
                    .any(|relation| relation.alias == next_filter.alias)
                {
                    return Err(invalid("JOIN EXISTS inner alias shadows an outer alias"));
                }
                let next_filter = filters.next().expect("peeked second shared EXISTS filter");
                let next_path = paths.next().expect("peeked second shared EXISTS path");
                result.extend(bind_shared_pair([filter, next_filter], path, outer)?);
                debug_assert_eq!(path, next_path);
                continue;
            }
        }
        let meta = inspect_query_source(path)?;
        let names = meta
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        let types = meta
            .columns
            .iter()
            .map(|column| column.data_type.clone())
            .collect::<Vec<_>>();
        let physical_columns = names.len();
        let inner = [Relation {
            alias: filter.alias.clone(),
            names,
            types,
            physical_columns,
            virtual_columns: Vec::new(),
            start: 0,
            rows: meta.rows,
        }];
        let inner_index = |column: &JoinColumn| {
            if column.relation != filter.alias {
                return Err(invalid("unknown JOIN EXISTS inner qualifier"));
            }
            column_index(&inner, column)
        };
        let pair =
            |(left, right): &(JoinColumn, JoinColumn)| -> Result<(usize, usize), ExecutionError> {
                let a = inner_index(left)?;
                let b = column_index(outer, right)?;
                let left_type = typed::base_type(column_type(&inner, a));
                let right_type = typed::base_type(column_type(outer, b));
                if left_type != right_type || !integer(left_type) {
                    return Err(invalid("JOIN EXISTS requires matching integer key types"));
                }
                Ok((a, b))
            };
        let (inner_eq, outer_eq) = pair(&filter.equality)?;
        let unequal = filter.inequality.as_ref().map(pair).transpose()?;
        let date_join = if let Some(date_filter) = &filter.date_filter {
            if outer
                .iter()
                .any(|relation| relation.alias == date_filter.alias)
            {
                return Err(invalid("JOIN EXISTS date alias shadows an outer alias"));
            }
            let date_path = paths
                .next()
                .ok_or_else(|| invalid("JOIN EXISTS is missing its date table path"))?;
            let date_meta = inspect_query_source(date_path)?;
            let date_column = date_meta
                .columns
                .iter()
                .find(|column| column.name.eq_ignore_ascii_case(&date_filter.keys.1.name))
                .ok_or_else(|| invalid("JOIN EXISTS date key column does not exist"))?;
            let date_type = typed::base_type(&date_column.data_type);
            if !integer(date_type) {
                return Err(invalid("JOIN EXISTS date key must be an integer"));
            }
            let fact_date_index = inner_index(&date_filter.keys.0)?;
            let fact_date_type = typed::base_type(column_type(&inner, fact_date_index));
            if fact_date_type != date_type {
                return Err(invalid("JOIN EXISTS fact/date keys require matching types"));
            }
            let predicate = RowPredicate::new(&date_filter.select, &date_meta.columns)?;
            let date_key_index = date_meta
                .columns
                .iter()
                .position(|column| column.name.eq_ignore_ascii_case(&date_column.name))
                .expect("date key column was validated");
            let date_fields = date_meta
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect::<Vec<_>>();
            let mut reader = TypedQueryReader::open(date_path, &date_fields)?;
            let mut allowed = HashSet::new();
            let mut rows = 0u64;
            while let Some(batch) = reader.next_batch()? {
                for index in 0..batch[0].len() {
                    rows = rows
                        .checked_add(1)
                        .ok_or_else(|| invalid("JOIN EXISTS date row count overflow"))?;
                    if predicate.matches(&batch, index) {
                        if let Some(key) = key(&typed::cell(&batch[date_key_index], index))? {
                            allowed.insert(key);
                            if allowed.len() > MAX_EXISTS_DATE_KEYS {
                                return Err(invalid(
                                    "JOIN EXISTS date key set exceeds 200000 keys",
                                ));
                            }
                        }
                    }
                }
            }
            if rows != date_meta.rows {
                return Err(invalid(
                    "JOIN EXISTS date input row count changed during scan",
                ));
            }
            Some((fact_date_index, allowed))
        } else {
            None
        };
        let mut comparisons = Vec::new();
        for (left, operator, right) in &filter.inner_comparisons {
            let a = inner_index(left)?;
            let b = inner_index(right)?;
            let ta = typed::base_type(column_type(&inner, a));
            let tb = typed::base_type(column_type(&inner, b));
            if ta != tb
                || !matches!(
                    operator,
                    crate::Operator::Eq
                        | crate::Operator::Ne
                        | crate::Operator::Lt
                        | crate::Operator::Le
                        | crate::Operator::Gt
                        | crate::Operator::Ge
                )
            {
                return Err(invalid(
                    "JOIN EXISTS inner comparison requires matching types",
                ));
            }
            comparisons.push((a, *operator, b));
        }
        let mut projected = vec![inner_eq];
        if let Some((index, _)) = unequal {
            projected.push(index);
        }
        if let Some((index, _)) = &date_join {
            projected.push(*index);
        }
        for (a, _, b) in &comparisons {
            projected.push(*a);
            projected.push(*b);
        }
        projected.sort_unstable();
        projected.dedup();
        let position = |index: usize| projected.binary_search(&index).expect("projected column");
        let date_position = date_join.as_ref().map(|(index, _)| position(*index));
        let fields = projected
            .iter()
            .map(|&index| inner[0].names[index].clone())
            .collect::<Vec<_>>();
        let mut reader = TypedQueryReader::open(path, &fields)?;
        let mut lookup = lookup_for(&inner, inner_eq, unequal.map(|(inner, _)| inner));
        let mut rows = 0u64;
        while let Some(batch) = reader.next_batch()? {
            for i in 0..batch[0].len() {
                rows = rows
                    .checked_add(1)
                    .ok_or_else(|| invalid("JOIN EXISTS row count overflow"))?;
                if let (Some(position), Some((_, date_keys))) = (&date_position, &date_join) {
                    let date_key = key(&typed::cell(&batch[*position], i))?;
                    if !date_key.is_some_and(|date_key| date_keys.contains(&date_key)) {
                        continue;
                    }
                }
                if !comparisons.iter().all(|(a, op, b)| {
                    typed::matches_comparison(
                        &typed::cell(&batch[position(*a)], i),
                        &typed::cell(&batch[position(*b)], i),
                        *op,
                    )
                }) {
                    continue;
                }
                let eq = key(&typed::cell(&batch[position(inner_eq)], i))?;
                let ne = unequal
                    .map(|(index, _)| key(&typed::cell(&batch[position(index)], i)))
                    .transpose()?
                    .flatten();
                lookup.insert(eq, ne)?;
            }
        }
        if rows != meta.rows {
            return Err(invalid("JOIN EXISTS input row count changed during scan"));
        }
        result.push(BoundExists {
            equality: outer_eq,
            inequality: unequal.map(|(_, index)| index),
            negated: filter.negated,
            lookup: BoundLookup::Single(lookup),
        });
    }
    if paths.next().is_some() {
        return Err(invalid("JOIN EXISTS has unused inner table paths"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(relation: &str, name: &str) -> JoinColumn {
        JoinColumn {
            relation: relation.into(),
            name: name.into(),
        }
    }

    fn exists_filter(alias: &str, negated: bool, require_late: bool) -> JoinExistsFilter {
        JoinExistsFilter {
            table: "lineitem".into(),
            alias: alias.into(),
            negated,
            equality: (column(alias, "order_id"), column("outer", "order_id")),
            inequality: Some((column(alias, "supp_id"), column("outer", "supp_id"))),
            inner_comparisons: if require_late {
                vec![(
                    column(alias, "receipt"),
                    crate::Operator::Gt,
                    column(alias, "commit"),
                )]
            } else {
                Vec::new()
            },
            date_filter: None,
        }
    }

    fn outer_row(order: u64, supplier: u64) -> [typed::Cell; 2] {
        [
            typed::Cell::Numeric(super::super::super::Number::Unsigned(order.into())),
            typed::Cell::Numeric(super::super::super::Number::Unsigned(supplier.into())),
        ]
    }

    #[test]
    fn null_outer_keys_do_not_match_exists_but_do_match_not_exists() {
        let mut lookup = JoinExistsLookup::new(true);
        lookup
            .insert(
                Some(JoinExistsKey::Unsigned(1)),
                Some(JoinExistsKey::Unsigned(2)),
            )
            .unwrap();
        let mut filter = BoundExists {
            equality: 0,
            inequality: Some(1),
            negated: false,
            lookup: BoundLookup::Single(lookup),
        };
        let one = typed::Cell::Numeric(super::super::super::Number::Unsigned(1));
        assert!(!filter.passes(&[typed::Cell::Null, one.clone()]).unwrap());
        assert!(!filter.passes(&[one.clone(), typed::Cell::Null]).unwrap());
        filter.negated = true;
        assert!(filter.passes(&[typed::Cell::Null, one.clone()]).unwrap());
        assert!(filter.passes(&[one, typed::Cell::Null]).unwrap());
    }

    #[test]
    fn filters_on_one_source_keep_independent_predicates() {
        use rusthouse_storage::{Column, write_wide_part};
        use std::fs;

        let part = std::env::temp_dir().join(format!(
            "rusthouse-correlated-exists-{}-{}",
            std::process::id(),
            line!(),
        ));
        let _ = fs::remove_dir_all(&part);
        let columns = [
            Column {
                name: "order_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "supp_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "receipt".into(),
                data_type: "Int64".into(),
            },
            Column {
                name: "commit".into(),
                data_type: "Int64".into(),
            },
            Column {
                name: "date_id".into(),
                data_type: "UInt64".into(),
            },
        ];
        let rows = [
            vec![
                Some("1".into()),
                Some("10".into()),
                Some("5".into()),
                Some("1".into()),
                Some("2020".into()),
            ],
            vec![
                Some("1".into()),
                Some("20".into()),
                Some("2".into()),
                Some("3".into()),
                Some("2021".into()),
            ],
            vec![
                Some("2".into()),
                Some("10".into()),
                Some("5".into()),
                Some("1".into()),
                Some("2020".into()),
            ],
        ];
        write_wide_part(&part, &columns, &rows).unwrap();
        let calendar = part.with_file_name(format!(
            "rusthouse-correlated-exists-calendar-{}-{}",
            std::process::id(),
            line!(),
        ));
        let _ = fs::remove_dir_all(&calendar);
        let calendar_columns = [
            Column {
                name: "date_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "fiscal_year".into(),
                data_type: "UInt64".into(),
            },
        ];
        let calendar_rows = [
            vec![Some("2020".into()), Some("1".into())],
            vec![Some("2021".into()), Some("2".into())],
        ];
        write_wide_part(&calendar, &calendar_columns, &calendar_rows).unwrap();
        let outer = [Relation {
            alias: "outer".into(),
            names: vec!["order_id".into(), "supp_id".into()],
            types: vec!["UInt64".into(), "UInt64".into()],
            physical_columns: 2,
            virtual_columns: Vec::new(),
            start: 0,
            rows: 0,
        }];
        let mut filters = [
            exists_filter("all_lines", false, false),
            exists_filter("late_lines", true, true),
        ];
        filters[0].date_filter = Some(crate::JoinExistsDateFilter {
            table: "calendar".into(),
            alias: "dates".into(),
            keys: (column("all_lines", "date_id"), column("dates", "date_id")),
            select: Box::new(
                crate::parse_select("SELECT date_id FROM calendar WHERE fiscal_year = 1").unwrap(),
            ),
        });
        let paths = [part.clone(), calendar.clone(), part.clone()];
        let bound = bind(&filters, &paths, &outer).unwrap();

        let order_one_supplier_ten = outer_row(1, 10);
        assert!(!bound[0].passes(&order_one_supplier_ten).unwrap());
        assert!(bound[1].passes(&order_one_supplier_ten).unwrap());

        let order_one_supplier_twenty = outer_row(1, 20);
        assert!(bound[0].passes(&order_one_supplier_twenty).unwrap());
        assert!(!bound[1].passes(&order_one_supplier_twenty).unwrap());

        let order_two_supplier_ten = outer_row(2, 10);
        assert!(!bound[0].passes(&order_two_supplier_ten).unwrap());
        assert!(bound[1].passes(&order_two_supplier_ten).unwrap());
        fs::remove_dir_all(calendar).unwrap();
        fs::remove_dir_all(part).unwrap();
    }

    #[test]
    fn fact_prefilters_only_mandatory_filters_with_fact_keys() {
        let filters = [
            BoundExists {
                equality: 0,
                inequality: Some(1),
                negated: false,
                lookup: BoundLookup::Single(JoinExistsLookup::new(true)),
            },
            BoundExists {
                equality: 0,
                inequality: None,
                negated: false,
                lookup: BoundLookup::Single(JoinExistsLookup::new(false)),
            },
            BoundExists {
                equality: 2,
                inequality: None,
                negated: false,
                lookup: BoundLookup::Single(JoinExistsLookup::new(false)),
            },
        ];
        assert_eq!(
            fact_prefilters(&filters, &[vec![0, 1], vec![0, 2]], 2).unwrap(),
            vec![0]
        );
        assert_eq!(fact_prefilters(&filters, &[], 2).unwrap(), vec![0, 1]);
        assert!(fact_prefilters(&filters, &[vec![3]], 2).is_err());
    }

    #[test]
    fn passes_expression_skips_only_prechecked_mandatory_filters() {
        let filters = [BoundExists {
            equality: 0,
            inequality: None,
            negated: false,
            lookup: BoundLookup::Single(JoinExistsLookup::new(false)),
        }];
        let row = [typed::Cell::Numeric(super::super::super::Number::Unsigned(
            1,
        ))];
        assert!(!passes_expression(&[vec![0]], &filters, &[], &row).unwrap());
        assert!(passes_expression(&[vec![0]], &filters, &[0], &row).unwrap());
    }

    #[test]
    fn same_source_exists_filters_share_the_scan_and_keep_results_independent() {
        use rusthouse_storage::{Column, write_wide_part};
        use std::fs;

        let part = std::env::temp_dir().join(format!(
            "rusthouse-shared-correlated-exists-{}-{}",
            std::process::id(),
            line!(),
        ));
        let _ = fs::remove_dir_all(&part);
        let columns = [
            Column {
                name: "order_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "supp_id".into(),
                data_type: "UInt64".into(),
            },
            Column {
                name: "receipt".into(),
                data_type: "Int64".into(),
            },
            Column {
                name: "commit".into(),
                data_type: "Int64".into(),
            },
        ];
        let rows = [
            vec![
                Some("1".into()),
                Some("10".into()),
                Some("5".into()),
                Some("1".into()),
            ],
            vec![
                Some("1".into()),
                Some("20".into()),
                Some("2".into()),
                Some("3".into()),
            ],
            vec![
                Some("1".into()),
                Some("30".into()),
                Some("6".into()),
                Some("2".into()),
            ],
            vec![
                Some("2".into()),
                Some("10".into()),
                Some("5".into()),
                Some("1".into()),
            ],
            vec![
                Some("2".into()),
                Some("20".into()),
                Some("2".into()),
                Some("3".into()),
            ],
            vec![
                Some("3".into()),
                Some("10".into()),
                Some("2".into()),
                Some("3".into()),
            ],
        ];
        write_wide_part(&part, &columns, &rows).unwrap();
        let outer = [Relation {
            alias: "outer".into(),
            names: vec!["order_id".into(), "supp_id".into()],
            types: vec!["UInt64".into(), "UInt64".into()],
            physical_columns: 2,
            virtual_columns: Vec::new(),
            start: 0,
            rows: 0,
        }];
        let filters = [
            exists_filter("all_lines", false, false),
            exists_filter("late_lines", true, true),
        ];
        let paths = [part.clone(), part.clone()];
        let bound = bind(&filters, &paths, &outer).unwrap();

        match (&bound[0].lookup, &bound[1].lookup) {
            (BoundLookup::Pair(first, 0), BoundLookup::Pair(second, 1)) => {
                assert!(Arc::ptr_eq(first, second));
            }
            _ => panic!("compatible EXISTS filters should share one lookup"),
        }

        assert!(bound[0].passes(&outer_row(2, 10)).unwrap());
        assert!(bound[1].passes(&outer_row(2, 10)).unwrap());
        assert!(bound[0].passes(&outer_row(1, 10)).unwrap());
        assert!(!bound[1].passes(&outer_row(1, 10)).unwrap());
        assert!(!bound[0].passes(&outer_row(3, 10)).unwrap());
        assert!(bound[1].passes(&outer_row(3, 10)).unwrap());

        fs::remove_dir_all(part).unwrap();
    }
}
