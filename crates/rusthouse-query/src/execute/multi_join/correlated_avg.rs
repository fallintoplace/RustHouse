//! Bounded per-key averages for a correlated join filter.
use super::*;
use crate::JoinCorrelatedAvg;
use rusthouse_storage::TypedQueryReader;

const MAX_AVG_KEYS: usize = 100_000;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum AvgKey {
    Signed(i128),
    Unsigned(u128),
    Text(Vec<u8>),
    Uuid(String),
}

fn avg_key(value: &typed::Cell) -> Result<Option<AvgKey>, ExecutionError> {
    match value {
        typed::Cell::Null => Ok(None),
        typed::Cell::Numeric(super::super::Number::Unsigned(value)) => {
            Ok(Some(AvgKey::Unsigned(*value)))
        }
        typed::Cell::Numeric(super::super::Number::Signed(value)) => {
            Ok(Some(AvgKey::Signed(*value)))
        }
        typed::Cell::Text(value) => Ok(Some(AvgKey::Text(value.clone()))),
        typed::Cell::Uuid(value) => Ok(Some(AvgKey::Uuid(value.clone()))),
        typed::Cell::Numeric(super::super::Number::Float(_)) => Err(invalid(
            "correlated AVG does not support floating-point keys",
        )),
    }
}

fn correlation_key_type(kind: &str) -> bool {
    super::exists::integer(kind) || matches!(kind, "String" | "UUID")
}

fn parse_output_key(data_type: &str, value: &str) -> Result<typed::Cell, ExecutionError> {
    if typed::base_type(data_type) == "String" {
        return rusthouse_format::decode_tsv_field(value.as_bytes())
            .map(typed::Cell::Text)
            .map_err(|_| invalid("invalid correlated AVG text key"));
    }
    typed::parse_literal(data_type, value)
}

pub(super) struct BoundAvg {
    pub(super) outer_key: usize,
    pub(super) outer_value: usize,
    operator: crate::Operator,
    values: HashMap<AvgKey, f64>,
}

fn numeric(kind: &str) -> bool {
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
            | "Float32"
            | "Float64"
    )
}

impl BoundAvg {
    pub(super) fn passes(&self, row: &[typed::Cell]) -> Result<bool, ExecutionError> {
        let Some(key) = avg_key(&row[self.outer_key])? else {
            return Ok(false);
        };
        let Some(value) = self.values.get(&key) else {
            return Ok(false);
        };
        let passes = typed::matches_comparison(
            &row[self.outer_value],
            &typed::Cell::Numeric(super::super::Number::Float(*value)),
            self.operator,
        );
        Ok(passes)
    }
}

pub(super) fn bind(
    plan: &JoinCorrelatedAvg,
    inner_paths: &[std::path::PathBuf],
    outer: &[Relation],
) -> Result<BoundAvg, ExecutionError> {
    let outer_key = column_index(outer, &plan.outer_key)?;
    let outer_value = column_index(outer, &plan.outer_value)?;
    let outer_key_type = typed::base_type(column_type(outer, outer_key));
    if !correlation_key_type(outer_key_type) {
        return Err(invalid(
            "correlated AVG requires integer, string, or UUID keys",
        ));
    }
    if !numeric(column_type(outer, outer_value)) {
        return Err(invalid("correlated AVG requires numeric values"));
    }
    let factor = plan
        .factor
        .parse::<f64>()
        .map_err(|_| invalid("invalid correlated AVG factor"))?;
    if !factor.is_finite() {
        return Err(invalid("invalid correlated AVG factor"));
    }
    let mut values = HashMap::new();
    if let Some(inner_query) = &plan.inner_join {
        if inner_paths.len() != inner_query.steps.len() + 2 {
            return Err(invalid(
                "correlated AVG inner source count differs from its plan",
            ));
        }
        let mut output = Vec::new();
        super::execute_multi_join(inner_query, inner_paths, &mut output)?;
        let output = String::from_utf8(output)
            .map_err(|_| invalid("correlated AVG inner output is not valid UTF-8"))?;
        let mut lines = output.lines();
        lines
            .next()
            .ok_or_else(|| invalid("correlated AVG inner query has no header"))?;
        for line in lines {
            let mut fields = line.split('\t');
            let key_text = fields
                .next()
                .ok_or_else(|| invalid("correlated AVG inner key is missing"))?;
            let value_text = fields
                .next()
                .ok_or_else(|| invalid("correlated AVG inner value is missing"))?;
            if fields.next().is_some() {
                return Err(invalid("correlated AVG inner query returned extra columns"));
            }
            if key_text == r"\N" || value_text == r"\N" {
                continue;
            }
            let key_cell = parse_output_key(column_type(outer, outer_key), key_text)?;
            let Some(key) = avg_key(&key_cell)? else {
                continue;
            };
            let average = value_text
                .parse::<f64>()
                .map_err(|_| invalid("correlated AVG inner value is not numeric"))?;
            let threshold = factor * average;
            if !threshold.is_finite() {
                return Err(invalid("correlated AVG result is not finite"));
            }
            if values.len() >= MAX_AVG_KEYS && !values.contains_key(&key) {
                return Err(invalid("correlated AVG supports at most 100000 keys"));
            }
            values.insert(key, threshold);
        }
    } else {
        let path = inner_paths
            .first()
            .ok_or_else(|| invalid("correlated AVG inner source is missing"))?;
        let metadata = inspect_query_source(path)?;
        let key = metadata
            .columns
            .iter()
            .position(|column| column.name == plan.inner_key)
            .ok_or_else(|| invalid("unknown correlated AVG inner key"))?;
        let value = metadata
            .columns
            .iter()
            .position(|column| column.name == plan.inner_value)
            .ok_or_else(|| invalid("unknown correlated AVG value"))?;
        let inner_key_kind = typed::base_type(&metadata.columns[key].data_type);
        if !correlation_key_type(inner_key_kind) || inner_key_kind != outer_key_type {
            return Err(invalid("correlated AVG requires matching key types"));
        }
        if !numeric(&metadata.columns[value].data_type) {
            return Err(invalid("correlated AVG requires numeric values"));
        }
        let fields = if key == value {
            vec![metadata.columns[key].name.clone()]
        } else {
            vec![
                metadata.columns[key].name.clone(),
                metadata.columns[value].name.clone(),
            ]
        };
        let value_position = usize::from(key != value);
        let mut reader = TypedQueryReader::open(path, &fields)?;
        let mut groups = HashMap::<AvgKey, (super::super::Number, u64)>::new();
        let mut rows = 0u64;
        while let Some(batch) = reader.next_batch()? {
            for index in 0..batch[0].len() {
                rows = rows
                    .checked_add(1)
                    .ok_or_else(|| invalid("correlated AVG row count overflow"))?;
                let Some(key) = avg_key(&typed::cell(&batch[0], index))? else {
                    continue;
                };
                let value = match typed::cell(&batch[value_position], index) {
                    typed::Cell::Null => continue,
                    typed::Cell::Numeric(value) => value,
                    _ => return Err(invalid("correlated AVG requires numeric values")),
                };
                if let Some((sum, count)) = groups.get_mut(&key) {
                    *sum = super::super::checked_aggregate_sum(*sum, value)?;
                    *count = count
                        .checked_add(1)
                        .ok_or_else(|| invalid("correlated AVG count overflow"))?;
                } else {
                    if groups.len() >= MAX_AVG_KEYS {
                        return Err(invalid("correlated AVG supports at most 100000 keys"));
                    }
                    groups.insert(key, (value, 1));
                }
            }
        }
        if rows != metadata.rows {
            return Err(invalid(
                "correlated AVG input row count changed during scan",
            ));
        }
        for (key, (sum, count)) in groups {
            let sum = match sum {
                super::super::Number::Unsigned(value) => value as f64,
                super::super::Number::Signed(value) => value as f64,
                super::super::Number::Float(value) => value,
            };
            let threshold = factor * (sum / count as f64);
            if !threshold.is_finite() {
                return Err(invalid("correlated AVG result is not finite"));
            }
            values.insert(key, threshold);
        }
    }
    Ok(BoundAvg {
        outer_key,
        outer_value,
        operator: plan.operator,
        values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_bounded_text_and_uuid_key_types() {
        assert!(correlation_key_type("String"));
        assert!(correlation_key_type("UUID"));
        assert!(!correlation_key_type("Float64"));
    }

    #[test]
    fn decodes_escaped_text_keys_from_inner_query_output() {
        let decoded = parse_output_key("Nullable(String)", r"New\tYork").unwrap();
        assert!(matches!(decoded, typed::Cell::Text(value) if value == b"New\tYork"));
        let escaped_slash = parse_output_key("String", r"New\\tYork").unwrap();
        assert!(matches!(escaped_slash, typed::Cell::Text(value) if value == b"New\\tYork"));
    }
}
