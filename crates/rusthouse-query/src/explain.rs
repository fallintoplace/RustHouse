use crate::{AggregateItem, Projection, Select, Source};

fn describe_source(source: Option<&Source>) -> String {
    match source {
        None => "constant row".to_owned(),
        Some(Source::Table(name)) => format!("table {name}"),
        Some(Source::Numbers(rows)) => format!("numbers({rows})"),
        Some(Source::NumbersRange { start, count }) => {
            format!("numbers({start}, {count})")
        }
        Some(Source::NumbersStepped {
            start,
            length,
            step,
        }) => format!("numbers({start}, {length}, {step})"),
        Some(Source::GenerateSeries { start, count, step }) => {
            if *count == 0 {
                format!("generate_series({start}, empty, {step})")
            } else {
                let last = u128::from(*start) + u128::from(*count - 1) * u128::from(*step);
                format!("generate_series({start}, {last}, {step})")
            }
        }
    }
}

fn join_labels(labels: impl Iterator<Item = String>) -> String {
    labels.collect::<Vec<_>>().join(", ")
}

fn describe_projection(projection: &Projection) -> String {
    match projection {
        Projection::Literal(value) => format!("Project: {value}"),
        Projection::Constants(columns) => format!(
            "Project: {}",
            join_labels(columns.iter().map(crate::Constant::header))
        ),
        Projection::All => "Project: *".to_owned(),
        Projection::Columns(columns) => format!("Project: {}", columns.join(", ")),
        Projection::Count => "Aggregate: count(*)".to_owned(),
        Projection::CountColumn { column } => format!("Aggregate: count({column})"),
        Projection::CountDistinct { column } => {
            format!("Aggregate: count(DISTINCT {column})")
        }
        Projection::ApproxDistinct { column } => {
            format!("Aggregate: uniq({column}) [HyperLogLog, 16384 registers]")
        }
        Projection::GroupedApproxDistinct { column, value_column } => {
            format!("Aggregate: GROUP BY {column}, uniq({value_column}) [bounded HyperLogLog]")
        }
        Projection::GroupedMultiApproxDistinct { columns, value_column } => {
            format!("Aggregate: GROUP BY {}, uniq({value_column}) [bounded HyperLogLog]", columns.join(", "))
        }
        Projection::GroupedCount { column } => {
            format!("Aggregate: GROUP BY {column}, count(*)")
        }
        Projection::GroupedDistinctCount {
            column,
            value_column,
        } => format!("Aggregate: GROUP BY {column}, count(DISTINCT {value_column})"),
        Projection::GroupedMultiDistinctCount { columns, value_column } => format!(
            "Aggregate: GROUP BY {}, count(DISTINCT {value_column})",
            columns.join(", ")
        ),
        Projection::GroupedMultiCount { columns } => {
            format!("Aggregate: GROUP BY {}, count(*)", columns.join(", "))
        }
        Projection::GroupedAggregate {
            column,
            function,
            value_column,
        } => format!(
            "Aggregate: GROUP BY {column}, {}({value_column})",
            function.name()
        ),
        Projection::GroupedAggregates { column, items, filters, .. } => format!(
            "Aggregate: GROUP BY {column}, {}",
            join_labels(items.iter().enumerate().map(|(index, (function, value))| {
                if let Some(filter) = filters[index].as_deref() {
                    format!("{}If({value}, {filter})", function.name())
                } else {
                    format!("{}({value})", function.name())
                }
            }))
        ),
        Projection::GroupedMultiAggregates { columns, items, filters, .. } => format!(
            "Aggregate: GROUP BY {}, {}",
            columns.join(", "),
            join_labels(items.iter().enumerate().map(|(index, (function, value))| {
                if let Some(filter) = filters[index].as_deref() {
                    format!("{}If({value}, {filter})", function.name())
                } else {
                    format!("{}({value})", function.name())
                }
            }))
        ),
        Projection::Aggregate { function, column } => {
            format!("Aggregate: {}({column})", function.name())
        }
        Projection::StringLength { column } => {
            format!("Project: length({column})")
        }
        Projection::MultipleAggregates(items) => {
            format!(
                "Aggregate: {}",
                join_labels(items.iter().map(AggregateItem::header))
            )
        }
    }
}

fn describe_sort_term(column: &str, descending: bool, nulls_first: Option<bool>) -> String {
    format!(
        "{} {}{}",
        column,
        if descending { "DESC" } else { "ASC" },
        match nulls_first {
            Some(true) => " NULLS FIRST",
            Some(false) => " NULLS LAST",
            None => "",
        }
    )
}

fn describe_sort(query: &Select) -> String {
    query
        .order_terms()
        .map(|order| {
            let label = if order.column == "#aggregate" {
                if let Projection::GroupedAggregate {
                    function,
                    value_column,
                    ..
                } = &query.projection
                {
                    format!("{}({value_column})", function.name())
                } else {
                    order.column.clone()
                }
            } else {
                order.column.clone()
            };
            describe_sort_term(&label, order.descending, order.nulls_first)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn describe_filter(query: &Select) -> Option<String> {
    if query.predicates.is_empty() {
        return None;
    }
    let count = query.predicates.len() + query.or_predicates.iter().map(Vec::len).sum::<usize>();
    Some(format!(
        "Filter: {} conditions across {} OR branches",
        count,
        query.or_predicates.len() + 1
    ))
}

/// Summarize parsed logical operators without reading data or estimating costs.
/// This is not a physical execution plan.
pub fn explain_select(query: &Select) -> String {
    let mut steps = Vec::new();
    steps.push(format!(
        "Source: {}",
        describe_source(query.source.as_ref())
    ));

    if let Some(filter) = describe_filter(query) {
        steps.push(filter);
    }

    steps.push(describe_projection(&query.projection));
    if let Some((source, target)) = &query.rename {
        steps.push(format!("Rename: {source} -> {target}"));
    }
    if let Some(alias) = &query.alias {
        steps.push(format!("Alias: {alias}"));
    }
    if let Some(having) = &query.having {
        let aggregate = match &query.projection {
            Projection::GroupedAggregate {
                function,
                value_column,
                ..
            } => format!("{}({value_column})", function.name()),
            _ => "count(*)".into(),
        };
        steps.push(format!(
            "Having: {aggregate} {:?} {}",
            having.operator, having.value
        ));
    }
    if query.distinct {
        steps.push("Distinct: projected rows".into());
    }
    if query.order_by.is_some() {
        let keys = describe_sort(query);
        steps.push(format!("Sort: {keys}"));
    }
    if let Some(offset) = query.offset {
        steps.push(format!("Offset: {offset}"));
    }
    if let Some(limit) = query.limit {
        steps.push(format!("Limit: {limit}"));
    }
    steps.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_select;

    #[test]
    fn labels_each_source_and_projection_without_query_execution() {
        let source = Source::NumbersRange {
            start: 5,
            count: 10,
        };
        assert_eq!(describe_source(Some(&source)), "numbers(5, 10)");
        assert_eq!(describe_source(None), "constant row");
        assert_eq!(describe_projection(&Projection::All), "Project: *");
        assert_eq!(
            describe_projection(&Projection::Count),
            "Aggregate: count(*)"
        );
    }

    #[test]
    fn explains_filter_sort_and_pagination_in_logical_order() {
        let query = parse_select(
            "SELECT number FROM numbers(20) WHERE number >= 3 OR number = 1 ORDER BY number DESC LIMIT 2 OFFSET 4",
        )
        .unwrap();
        assert_eq!(
            explain_select(&query),
            "Source: numbers(20)\nFilter: 2 conditions across 2 OR branches\nProject: number\nSort: number DESC\nOffset: 4\nLimit: 2"
        );
    }

    #[test]
    fn explains_wildcard_rename_without_scanning_rows() {
        let query = parse_select("SELECT * RENAME number AS value FROM numbers(3)").unwrap();
        assert_eq!(
            explain_select(&query),
            "Source: numbers(3)\nProject: *\nRename: number -> value"
        );
    }

    #[test]
    fn describes_explicit_null_sort_placement() {
        let query =
            parse_select("SELECT number FROM numbers(3) ORDER BY number DESC NULLS LAST").unwrap();
        assert!(explain_select(&query).contains("Sort: number DESC NULLS LAST"));
    }

    #[test]
    fn explains_groups_aggregates_and_constants() {
        let grouped =
            parse_select("SELECT id, count(*) FROM part GROUP BY id HAVING count(*) >= 2").unwrap();
        assert!(explain_select(&grouped).contains("Aggregate: GROUP BY id, count(*)"));
        assert!(explain_select(&grouped).contains("Having: count(*) Ge 2"));
        let literal = parse_select("SELECT 42 AS answer").unwrap();
        assert_eq!(
            explain_select(&literal),
            "Source: constant row\nProject: 42\nAlias: answer"
        );
    }
}
