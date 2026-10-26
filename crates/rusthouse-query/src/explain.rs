use crate::{Projection, Select, Source};

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

fn describe_projection(projection: &Projection) -> String {
    match projection {
        Projection::Literal(value) => format!("Project: {value}"),
        Projection::All => "Project: *".to_owned(),
        Projection::Columns(columns) => format!("Project: {}", columns.join(", ")),
        Projection::Count => "Aggregate: count(*)".to_owned(),
        Projection::CountColumn { column } => format!("Aggregate: count({column})"),
        Projection::CountDistinct { column } => {
            format!("Aggregate: count(DISTINCT {column})")
        }
        Projection::GroupedCount { column } => {
            format!("Aggregate: GROUP BY {column}, count(*)")
        }
        Projection::Aggregate { function, column } => {
            format!("Aggregate: {}({column})", function.name())
        }
    }
}

/// Summarize parsed logical operators without reading data or estimating costs.
/// This is not a physical execution plan.
pub fn explain_select(query: &Select) -> String {
    let mut steps = Vec::new();
    steps.push(format!(
        "Source: {}",
        describe_source(query.source.as_ref())
    ));

    if !query.predicates.is_empty() {
        let count =
            query.predicates.len() + query.or_predicates.iter().map(Vec::len).sum::<usize>();
        steps.push(format!(
            "Filter: {} conditions across {} OR branches",
            count,
            query.or_predicates.len() + 1
        ));
    }

    steps.push(describe_projection(&query.projection));
    if let Some(alias) = &query.alias {
        steps.push(format!("Alias: {alias}"));
    }
    if let Some(having) = &query.having {
        steps.push(format!(
            "Having: count(*) {:?} {}",
            having.operator, having.value
        ));
    }
    if query.distinct {
        steps.push("Distinct: projected rows".into());
    }
    if let Some(order) = &query.order_by {
        steps.push(format!(
            "Sort: {} {}{}",
            order.column,
            if order.descending { "DESC" } else { "ASC" },
            match order.nulls_first {
                Some(true) => " NULLS FIRST",
                Some(false) => " NULLS LAST",
                None => "",
            }
        ));
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
