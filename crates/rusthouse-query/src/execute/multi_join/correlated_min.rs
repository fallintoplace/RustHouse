//! Bounded per-key minimums for a correlated join filter.
use super::*;
use crate::{JoinCorrelatedMin, JoinExistsKey, Operator};
use std::io;

const MAX_MIN_BYTES: usize = 16 * 1024 * 1024;
const MAX_MIN_KEYS: usize = 100_000;

struct LimitedRows(Vec<u8>);
impl Write for LimitedRows {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_MIN_BYTES {
            return Err(io::Error::other("correlated MIN result exceeds 16 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) struct BoundMin {
    pub(super) outer_key: usize,
    pub(super) outer_value: usize,
    values: HashMap<JoinExistsKey, typed::Cell>,
}
impl BoundMin {
    pub(super) fn passes(&self, row: &[typed::Cell]) -> Result<bool, ExecutionError> {
        let Some(key) = super::exists::key(&row[self.outer_key])? else {
            return Ok(false);
        };
        let Some(value) = self.values.get(&key) else {
            return Ok(false);
        };
        Ok(typed::matches_comparison(
            &row[self.outer_value],
            value,
            Operator::Eq,
        ))
    }
}

pub(super) fn bind(
    plan: &JoinCorrelatedMin,
    paths: &[PathBuf],
    outer: &[Relation],
    skip_rows: bool,
) -> Result<BoundMin, ExecutionError> {
    if paths.len() != plan.inner.steps.len() + 2 {
        return Err(invalid("correlated MIN input count differs from its plan"));
    }
    let mut relations = Vec::new();
    let mut width = 0usize;
    for (index, path) in paths.iter().enumerate() {
        let metadata = inspect_query_source(path)?;
        let alias = match index {
            0 => plan.inner.left_alias.clone(),
            1 => plan.inner.right_alias.clone(),
            _ => plan.inner.steps[index - 2].alias.clone(),
        };
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
        let physical_columns = names.len();
        relations.push(Relation {
            alias,
            start: width,
            names,
            types,
            physical_columns,
            virtual_columns: Vec::new(),
            rows: metadata.rows,
        });
        width = width
            .checked_add(metadata.columns.len())
            .ok_or_else(|| invalid("correlated MIN column count overflow"))?;
    }
    let inner_key = column_index(&relations, &plan.inner_key)?;
    let inner_value = column_index(&relations, &plan.inner_value)?;
    let outer_key = column_index(outer, &plan.outer_key)?;
    let outer_value = column_index(outer, &plan.outer_value)?;
    let key_type = typed::base_type(column_type(&relations, inner_key));
    let value_type = typed::base_type(column_type(&relations, inner_value));
    if !super::exists::integer(key_type)
        || key_type != typed::base_type(column_type(outer, outer_key))
    {
        return Err(invalid("correlated MIN requires matching integer keys"));
    }
    if value_type != typed::base_type(column_type(outer, outer_value))
        || !matches!(
            value_type,
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
                | "Float32"
                | "Float64"
        )
    {
        return Err(invalid("correlated MIN requires matching numeric values"));
    }
    let mut output = LimitedRows(Vec::new());
    let mut inner = plan.inner().clone();
    if skip_rows {
        inner.limit = Some(0);
    }
    super::execute_multi_join(&inner, paths, &mut output)?;
    let text =
        std::str::from_utf8(&output.0).map_err(|_| invalid("invalid correlated MIN output"))?;
    let mut lines = text.lines();
    lines
        .next()
        .ok_or_else(|| invalid("missing correlated MIN header"))?;
    let mut values = HashMap::new();
    for line in lines {
        let (key, value) = line
            .split_once('\t')
            .ok_or_else(|| invalid("invalid correlated MIN row"))?;
        if key == "\\N" || value == "\\N" {
            continue;
        }
        let key = typed::parse_literal(key_type, key)?;
        let Some(key) = super::exists::key(&key)? else {
            continue;
        };
        let value = typed::parse_literal(value_type, value)?;
        if values.len() >= MAX_MIN_KEYS {
            return Err(invalid("correlated MIN supports at most 100000 keys"));
        }
        if values.insert(key, value).is_some() {
            return Err(invalid("duplicate correlated MIN key"));
        }
    }
    Ok(BoundMin {
        outer_key,
        outer_value,
        values,
    })
}
