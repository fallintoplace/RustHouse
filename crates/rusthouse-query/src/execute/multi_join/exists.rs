//! Build and apply bounded correlated EXISTS lookups.
use super::*;
use crate::{JoinExistsFilter, JoinExistsKey, JoinExistsLookup, RowPredicate};
use rusthouse_storage::{TypedQueryReader, inspect_query_source};
use std::collections::HashSet;

const MAX_EXISTS_DATE_KEYS: usize = 200_000;

pub(super) struct BoundExists {
    pub(super) equality: usize,
    pub(super) inequality: Option<usize>,
    pub(super) negated: bool,
    lookup: JoinExistsLookup,
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
        Ok(self.lookup.exists(equal, unequal) != self.negated)
    }
}

pub(super) fn passes_expression(
    expression: &[Vec<usize>],
    filters: &[BoundExists],
    row: &[typed::Cell],
) -> Result<bool, ExecutionError> {
    if expression.is_empty() {
        for filter in filters {
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
    let mut paths = paths.iter();
    for filter in filters {
        if outer.iter().any(|relation| relation.alias == filter.alias) {
            return Err(invalid("JOIN EXISTS inner alias shadows an outer alias"));
        }
        let path = paths
            .next()
            .ok_or_else(|| invalid("JOIN EXISTS is missing its fact table path"))?;
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
            lookup,
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
            lookup,
        };
        let one = typed::Cell::Numeric(super::super::super::Number::Unsigned(1));
        assert!(!filter.passes(&[typed::Cell::Null, one.clone()]).unwrap());
        assert!(!filter.passes(&[one.clone(), typed::Cell::Null]).unwrap());
        filter.negated = true;
        assert!(filter.passes(&[typed::Cell::Null, one.clone()]).unwrap());
        assert!(filter.passes(&[one, typed::Cell::Null]).unwrap());
    }
}
