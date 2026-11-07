//! Selection of typed/numeric physical scan paths and numeric projection setup.
//! Keep specialized operators before this fallback, as in the original executor.
use super::*;
use super::predicates::CompiledPredicate;

pub(super) enum StorageScanPlan {
    Numeric,
    Typed,
    TypedAggregate,
}

fn select_typed_route(projection: &Projection, needs_typed: bool) -> StorageScanPlan {
    if !needs_typed {
        StorageScanPlan::Numeric
    } else if matches!(projection, Projection::Aggregate { .. }) {
        StorageScanPlan::TypedAggregate
    } else {
        StorageScanPlan::Typed
    }
}

pub(super) fn select_storage_scan(
    query: &Select,
    part: &Path,
) -> Result<StorageScanPlan, ExecutionError> {
    let needs_typed = typed::requires_typed_scan(query, part)?;
    Ok(select_typed_route(&query.projection, needs_typed))
}

pub(super) struct NumericScanPlan {
    pub(super) projected: Vec<String>,
    pub(super) projected_indices: Vec<usize>,
    pub(super) scan_columns: Vec<String>,
    pub(super) comparisons: Vec<Vec<CompiledPredicate>>,
}

fn deduplicate_scan_columns(projected: &[String]) -> (Vec<String>, Vec<usize>) {
    let mut scan_columns = Vec::new();
    let projected_indices = projected
        .iter()
        .map(|name| {
            if let Some(index) = scan_columns.iter().position(|column| column == name) {
                index
            } else {
                let index = scan_columns.len();
                scan_columns.push(name.clone());
                index
            }
        })
        .collect::<Vec<_>>();
(scan_columns, projected_indices)
}

pub(super) fn prepare_numeric_scan(
    query: &Select,
    metadata: &rusthouse_storage::PartMetadata,
) -> Result<NumericScanPlan, ExecutionError> {
    let projected = match &query.projection {
        Projection::All => metadata
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>(),
        Projection::Columns(names) => names.clone(),
        Projection::Count => Vec::new(),
        Projection::Aggregate { .. }
        | Projection::CountColumn { .. }
        | Projection::CountDistinct { .. }
        | Projection::ApproxDistinct { .. }
        | Projection::Histogram { .. }
        | Projection::ApproxDistinctState { .. }
        | Projection::GroupedCount { .. }
        | Projection::GroupedDistinctCount { .. }
        | Projection::GroupedMultiDistinctCount { .. }
        | Projection::GroupedMultiCount { .. }
        | Projection::GroupedAggregate { .. }
        | Projection::GroupedAggregates { .. }
        | Projection::GroupedMultiAggregates { .. }
        | Projection::MultipleAggregates(_)
        | Projection::StringLength { .. }
        | Projection::Literal(_)
        | Projection::Constants(_) => unreachable!(),
    };
    if query.distinct
        && query.order_terms().any(|order| {
            !projected
                .iter()
                .any(|column| column == query.order_source_column(&order.column))
        })
    {
        return Err(ExecutionError::Invalid(
            "DISTINCT ORDER BY column must appear in SELECT".into(),
        ));
    }
    let (mut scan_columns, projected_indices) = deduplicate_scan_columns(&projected);
    let comparisons = compile_part_where(query, &mut scan_columns, |column_name, literal| {
        let column = metadata
            .columns
            .iter()
            .find(|column| column.name == column_name)
            .ok_or_else(|| {
                ExecutionError::Invalid(format!("unknown WHERE column: {column_name}"))
            })?;
        parse_numeric(&column.data_type, literal)
    })?;
    Ok(NumericScanPlan {
        projected,
        projected_indices,
        scan_columns,
        comparisons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_projections_reuse_scanned_columns() {
        let projected = vec!["id".into(), "value".into(), "id".into()];
        let (columns, indices) = deduplicate_scan_columns(&projected);
        assert_eq!(columns, vec!["id".to_string(), "value".to_string()]);
        assert_eq!(indices, vec![0, 1, 0]);
    }

    #[test]
    fn typed_route_only_changes_aggregate_dispatch() {
        assert!(matches!(
            select_typed_route(&Projection::Count, false),
            StorageScanPlan::Numeric
        ));
        assert!(matches!(
            select_typed_route(&Projection::Count, true),
            StorageScanPlan::Typed
        ));
        assert!(matches!(
            select_typed_route(
                &Projection::Aggregate {
                    function: crate::AggregateFunction::Sum,
                    column: "value".into(),
                },
                true,
            ),
            StorageScanPlan::TypedAggregate
        ));
    }
}
