//! Shared binding and comparison evaluation for conditional grouped metrics.
use super::{ExecutionError, typed};
use crate::{GroupedFilter, Operator};
use rusthouse_storage::TypedColumn;
use std::collections::HashMap;

pub(super) enum BoundGroupedFilter {
    Truthy(usize),
    Comparison {
        index: usize,
        operator: Operator,
        literal: typed::Cell,
    },
    Like {
        index: usize,
        pattern: typed::TextPattern,
    },
}

pub(super) fn bind_grouped_filters(
    filters: &[Option<GroupedFilter>],
    indices: &HashMap<String, usize>,
    types: &HashMap<String, String>,
) -> Result<Vec<Option<BoundGroupedFilter>>, ExecutionError> {
    filters
        .iter()
        .map(|filter| {
            let Some(filter) = filter else {
                return Ok(None);
            };
            let name = filter.column();
            let index = *indices.get(name).ok_or_else(|| {
                ExecutionError::Invalid(format!("unknown grouped filter column: {name}"))
            })?;
            match filter {
                GroupedFilter::Truthy(_) => Ok(Some(BoundGroupedFilter::Truthy(index))),
                GroupedFilter::CaseLike { pattern, .. } => {
                    let data_type = types.get(name).ok_or_else(|| {
                        ExecutionError::Invalid(format!("unknown grouped filter column: {name}"))
                    })?;
                    if typed::base_type(data_type) != "String" {
                        return Err(ExecutionError::Invalid(
                            "grouped CASE LIKE requires a String column".into(),
                        ));
                    }
                    Ok(Some(BoundGroupedFilter::Like {
                        index,
                        pattern: typed::compile_text_pattern(pattern)?,
                    }))
                }
                GroupedFilter::Comparison { operator, literal, .. }
                | GroupedFilter::CaseElseZero { operator, literal, .. } => {
                    let data_type = types.get(name).ok_or_else(|| {
                        ExecutionError::Invalid(format!("unknown grouped filter column: {name}"))
                    })?;
                    let literal = typed::parse_literal(data_type, literal)?;
                    Ok(Some(BoundGroupedFilter::Comparison { index, operator: *operator, literal }))
                }
            }
        })
        .collect()
}

impl BoundGroupedFilter {
    pub(super) fn passes(
        &self,
        batch: &[TypedColumn],
        row: usize,
        truthy: fn(&TypedColumn, usize) -> Result<bool, ExecutionError>,
    ) -> Result<bool, ExecutionError> {
        match self {
            Self::Truthy(index) => truthy(&batch[*index], row),
            Self::Like { index, pattern } => Ok(match typed::cell(&batch[*index], row) {
                typed::Cell::Text(value) => typed::text_pattern_matches(&value, pattern, false),
                _ => false,
            }),
            Self::Comparison { index, operator, literal } => Ok(typed::matches_comparison(
                &typed::cell(&batch[*index], row),
                literal,
                *operator,
            )),
        }
    }
}
