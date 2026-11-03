//! Build and apply bounded correlated EXISTS lookups.
use super::*;
use crate::{JoinExistsFilter, JoinExistsKey, JoinExistsLookup};
use rusthouse_storage::TypedQueryReader;

pub(super) struct BoundExists {
    pub(super) equality: usize,
    pub(super) inequality: Option<usize>,
    pub(super) negated: bool,
    lookup: JoinExistsLookup,
}

fn integer(kind: &str) -> bool {
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

fn key(value: &typed::Cell) -> Result<Option<JoinExistsKey>, ExecutionError> {
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

pub(super) fn bind(
    filters: &[JoinExistsFilter],
    paths: &[PathBuf],
    outer: &[Relation],
) -> Result<Vec<BoundExists>, ExecutionError> {
    if filters.is_empty() || filters.len() > 2 || paths.len() != filters.len() {
        return Err(invalid("JOIN EXISTS requires one path per filter"));
    }
    let mut result = Vec::with_capacity(filters.len());
    for (filter, path) in filters.iter().zip(paths) {
        if outer.iter().any(|relation| relation.alias == filter.alias) {
            return Err(invalid("JOIN EXISTS inner alias shadows an outer alias"));
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
        let inner = [Relation {
            alias: filter.alias.clone(),
            names,
            types,
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
        for (a, _, b) in &comparisons {
            projected.push(*a);
            projected.push(*b);
        }
        projected.sort_unstable();
        projected.dedup();
        let position = |index: usize| projected.binary_search(&index).expect("projected column");
        let fields = projected
            .iter()
            .map(|&index| inner[0].names[index].clone())
            .collect::<Vec<_>>();
        let mut reader = TypedQueryReader::open(path, &fields)?;
        let mut lookup = JoinExistsLookup::new(unequal.is_some());
        let mut rows = 0u64;
        while let Some(batch) = reader.next_batch()? {
            for i in 0..batch[0].len() {
                rows = rows
                    .checked_add(1)
                    .ok_or_else(|| invalid("JOIN EXISTS row count overflow"))?;
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
