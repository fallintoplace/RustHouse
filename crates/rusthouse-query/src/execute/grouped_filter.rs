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
    Combined {
        index: usize,
        comparisons: Vec<(Operator, typed::Cell)>,
        all: bool,
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
                GroupedFilter::CaseElseZeroCombined {
                    comparisons, all, ..
                } => {
                    let data_type = types.get(name).ok_or_else(|| {
                        ExecutionError::Invalid(format!("unknown grouped filter column: {name}"))
                    })?;
                    let comparisons = comparisons
                        .iter()
                        .map(|(operator, literal)| {
                            Ok((*operator, typed::parse_literal(data_type, literal)?))
                        })
                        .collect::<Result<Vec<_>, ExecutionError>>()?;
                    Ok(Some(BoundGroupedFilter::Combined {
                        index,
                        comparisons,
                        all: *all,
                    }))
                }
                GroupedFilter::Comparison {
                    operator, literal, ..
                }
                | GroupedFilter::CaseElseZero {
                    operator, literal, ..
                } => {
                    let data_type = types.get(name).ok_or_else(|| {
                        ExecutionError::Invalid(format!("unknown grouped filter column: {name}"))
                    })?;
                    let literal = typed::parse_literal(data_type, literal)?;
                    Ok(Some(BoundGroupedFilter::Comparison {
                        index,
                        operator: *operator,
                        literal,
                    }))
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
            Self::Combined {
                index,
                comparisons,
                all,
            } => {
                let value = typed::cell(&batch[*index], row);
                Ok(if *all {
                    comparisons.iter().all(|(operator, literal)| {
                        typed::matches_comparison(&value, literal, *operator)
                    })
                } else {
                    comparisons.iter().any(|(operator, literal)| {
                        typed::matches_comparison(&value, literal, *operator)
                    })
                })
            }
            Self::Comparison {
                index,
                operator,
                literal,
            } => Ok(typed::matches_comparison(
                &typed::cell(&batch[*index], row),
                literal,
                *operator,
            )),
        }
    }
}
