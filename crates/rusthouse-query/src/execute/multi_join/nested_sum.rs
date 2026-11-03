//! Execute a bounded nested IN over a correlated SUM join.
use super::*;
use crate::{NestedJoinSum, Predicate};
use std::collections::HashSet;
use std::io;

const MAX_NESTED_BYTES: usize = 16 * 1024 * 1024;
const MAX_NESTED_KEYS: usize = 100_000;

struct LimitedRows(Vec<u8>);
impl Write for LimitedRows {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_NESTED_BYTES {
            return Err(io::Error::other("nested IN result exceeds 16 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn relations(query: &JoinSelect, paths: &[PathBuf]) -> Result<Vec<Relation>, ExecutionError> {
    if paths.len() != query.steps.len() + 2 {
        return Err(invalid("nested IN source count differs from its plan"));
    }
    let mut relations = Vec::new();
    let mut width = 0usize;
    for (index, path) in paths.iter().enumerate() {
        let metadata = inspect_query_source(path)?;
        let alias = match index {
            0 => query.left_alias.clone(),
            1 => query.right_alias.clone(),
            _ => query.steps[index - 2].alias.clone(),
        };
        let names = metadata
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect();
        let types = metadata
            .columns
            .iter()
            .map(|column| column.data_type.clone())
            .collect();
        relations.push(Relation {
            alias,
            names,
            types,
            start: width,
            rows: metadata.rows,
        });
        width = width
            .checked_add(metadata.columns.len())
            .ok_or_else(|| invalid("nested IN column count overflow"))?;
    }
    Ok(relations)
}

fn check_filter_budget(values: &[String], branches: usize) -> Result<(), ExecutionError> {
    values
        .iter()
        .try_fold(0usize, |total, value| {
            total.checked_add(value.len().saturating_add(192))
        })
        .and_then(|size| size.checked_mul(branches))
        .filter(|size| *size <= MAX_NESTED_BYTES)
        .ok_or_else(|| invalid("nested IN filters exceed 16 MiB"))?;
    Ok(())
}

/// Execute while the caller holds the database read lock for all inputs.
pub fn execute_nested_join_sum<W: Write>(
    plan: &NestedJoinSum,
    outer_paths: &[PathBuf],
    inner_paths: &[PathBuf],
    sum_path: &std::path::Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let outer_relations = relations(plan.outer(), outer_paths)?;
    let inner_relations = relations(plan.inner().outer(), inner_paths)?;
    let outer_key = column_index(&outer_relations, &plan.outer_key)?;
    let inner_key = column_index(&inner_relations, &plan.inner_key)?;
    let kind = typed::base_type(column_type(&inner_relations, inner_key));
    if !super::exists::integer(kind)
        || kind != typed::base_type(column_type(&outer_relations, outer_key))
    {
        return Err(invalid("nested IN requires matching integer keys"));
    }
    let mut inner = plan.inner().clone();
    if plan.outer().limit == Some(0) {
        inner.cap_rows(0);
    }
    let mut result = LimitedRows(Vec::new());
    super::execute_nested_correlated_sum(&inner, inner_paths, sum_path, &mut result)?;
    let text = std::str::from_utf8(&result.0).map_err(|_| invalid("invalid nested IN output"))?;
    let mut lines = text.lines();
    lines
        .next()
        .ok_or_else(|| invalid("missing nested IN header"))?;
    let mut distinct = HashSet::new();
    for value in lines {
        if value == "\\N" {
            continue;
        }
        if value.contains('\t') {
            return Err(invalid("invalid nested IN row"));
        }
        typed::parse_literal(kind, value)?;
        if !distinct.contains(value) && distinct.len() >= MAX_NESTED_KEYS {
            return Err(invalid("nested IN supports at most 100000 keys"));
        }
        distinct.insert(value.to_owned());
    }
    let values = distinct.into_iter().collect::<Vec<_>>();
    let name = if plan.outer_key.relation.is_empty() {
        plan.outer_key.name.clone()
    } else {
        format!("{}\0{}", plan.outer_key.relation, plan.outer_key.name)
    };
    let mut outer = plan.outer().clone();
    let branch_count = outer.scalar_where.as_ref().map_or(1, Vec::len);
    check_filter_budget(&values, branch_count)?;
    let branches = outer.scalar_where.get_or_insert_with(|| vec![vec![]]);
    for branch in branches {
        branch.push(Predicate::In {
            column: name.clone(),
            values: values.clone(),
        });
    }
    super::execute_multi_join(&outer, outer_paths, output)
}

#[cfg(test)]
mod tests {
    use super::check_filter_budget;

    #[test]
    fn bounds_membership_copies_across_boolean_branches() {
        let values = vec!["123".to_owned(); 2048];
        assert!(check_filter_budget(&values, 1).is_ok());
        assert!(check_filter_budget(&values, 64).is_err());
        assert!(check_filter_budget(&values, usize::MAX).is_err());
        assert!(check_filter_budget(&[], 64).is_ok());
    }
}
