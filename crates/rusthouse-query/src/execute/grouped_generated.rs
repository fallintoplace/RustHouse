//! Fused group metrics over generated numeric series, without stored parts.
use super::{AggregateState, ExecutionError, Number, compile_numbers_predicates, number_matches_all, typed};
use crate::{AggregateFunction, Select};
use std::cmp::Ordering;
use std::io::Write;

const MAX_GROUPS: usize = 200_000;
const MAX_GENERATED_GROUP_BYTES: usize = 64 * 1024 * 1024;

/// Each generated group owns one aggregate-state vector. The group-count
/// ceiling alone cannot bound memory when a query selects many metrics.
fn charge_group_states(
    used: &mut usize,
    items: &[(AggregateFunction, String)],
) -> Result<(), ExecutionError> {
    let state_bytes = items.len().checked_mul(std::mem::size_of::<AggregateState>())
        .ok_or_else(|| ExecutionError::Invalid("generated group memory overflow".into()))?;
    let median_slots = items.iter().filter(|(function, _)| {
        *function == AggregateFunction::Median
    }).count().checked_mul(4 * std::mem::size_of::<Number>())
        .ok_or_else(|| ExecutionError::Invalid("generated group memory overflow".into()))?;
    let next = used.checked_add(std::mem::size_of::<(u128, Vec<AggregateState>)>())
        .and_then(|bytes| bytes.checked_add(state_bytes))
        .and_then(|bytes| bytes.checked_add(median_slots))
        .ok_or_else(|| ExecutionError::Invalid("generated group memory overflow".into()))?;
    if next > MAX_GENERATED_GROUP_BYTES {
        return Err(ExecutionError::Invalid(
            "generated GROUP BY exceeds 64 MiB state budget".into()
        ));
    }
    *used = next;
    Ok(())
}

pub(super) fn execute<W: Write>(
    query: &Select,
    start: u64,
    rows: u64,
    step: u64,
    column: &str,
    items: &[(AggregateFunction, String)],
    aliases: &[Option<String>],
    output: &mut W,
) -> Result<(), ExecutionError> {
    if column != "number" || items.iter().any(|(_, name)| name != "number") {
        return Err(ExecutionError::Invalid(
            "generated grouped analytics supports only the number column".into(),
        ));
    }
    write!(output, "{column}")?;
    for (index, (function, argument)) in items.iter().enumerate() {
        if let Some(alias) = aliases.get(index).and_then(Option::as_deref) {
            write!(output,"\t{alias}")?;
        } else {
            write!(output,"\t{}({argument})",function.name())?;
        }
    }
    writeln!(output)?;
    if query.limit == Some(0) {return Ok(());}
    let comparisons=compile_numbers_predicates(query)?;
    // Generated number keys are unique. For an unordered bounded page, jump
    // straight to the requested range instead of scanning billions of rows.
    if query.order_by.is_none() && query.having.is_none()
        && query.limit.is_some_and(|limit| limit <= MAX_GROUPS)
    {
        let range = if query.predicates.is_empty() && query.or_predicates.is_empty() {
            Some((0, rows))
        } else {
            super::monotonic_range_indices(&comparisons, start, rows, step)
        };
        if let (Some((low, high)), Some(limit)) = (range, query.limit) {
            let first = low.saturating_add(query.offset.unwrap_or(0) as u64).min(high);
            let last = first.saturating_add(limit as u64).min(high);
            for index in first..last {
                let value = u128::from(start) + u128::from(index) * u128::from(step);
                write!(output, "{value}")?;
                for (function, _) in items {
                    let mut state = AggregateState::new(*function);
                    state.update(if *function == AggregateFunction::Count {
                        Number::Unsigned(1)
                    } else {
                        Number::Unsigned(value)
                    })?;
                    write!(output, "\t{}", state.render())?;
                }
                writeln!(output)?;
            }
            return Ok(());
        }
    }
    if query.limit.is_none() && query.predicates.is_empty()
        && query.or_predicates.is_empty() && query.having.is_none()
        && query.order_by.is_none() && query.order_by_extra.is_empty()
        && step != 0
    {
        if rows > MAX_GROUPS as u64 {
            return Err(ExecutionError::Invalid(
                "generated GROUP BY supports at most 200000 groups".into(),
            ));
        }
        let mut per_group = 0usize;
        charge_group_states(&mut per_group, items)?;
        if (rows as usize).checked_mul(per_group)
            .is_none_or(|total| total > MAX_GENERATED_GROUP_BYTES)
        {
            return Err(ExecutionError::Invalid(
                "generated GROUP BY exceeds 64 MiB state budget".into(),
            ));
        }
        for index in query.offset.unwrap_or(0) as u64..rows {
            let value = u128::from(start) + u128::from(index) * u128::from(step);
            write!(output, "{value}")?;
            for (function, _) in items {
                let mut state = AggregateState::new(*function);
                state.update(if *function == AggregateFunction::Count {
                    Number::Unsigned(1)
                } else {
                    Number::Unsigned(value)
                })?;
                write!(output, "\t{}", state.render())?;
            }
            writeln!(output)?;
        }
        return Ok(());
    }
    let mut groups=Vec::<(u128,Vec<AggregateState>)>::new();
    let mut retained_state_bytes = 0usize;
    for index in 0..rows {
        let value=u128::from(start)+u128::from(index)*u128::from(step);
        if !number_matches_all(Number::Unsigned(value),&comparisons) {continue;}
        if groups.len()>=MAX_GROUPS {
            return Err(ExecutionError::Invalid(
                "generated GROUP BY supports at most 200000 groups".into()));
        }
        // The virtual number column is unique for nonzero steps. Every row
        // forms one group and can share the regular aggregate-state semantics.
        charge_group_states(&mut retained_state_bytes, items)?;
        let mut states=Vec::with_capacity(items.len());
        for (function,_) in items {
            let mut state=AggregateState::new(*function);
            state.update(if *function == AggregateFunction::Count {
                Number::Unsigned(1)
            } else {
                Number::Unsigned(value)
            })?;
            states.push(state);
        }
        groups.push((value,states));
    }
    // HAVING applies to aggregate results before ORDER BY and pagination.
    if let Some(having) = &query.having {
        let index = having.aggregate_index.ok_or_else(|| {
            ExecutionError::Invalid("HAVING metric index is required".into())
        })?;
        groups.retain(|(_, states)| states.get(index)
            .is_some_and(|state| state.passes_having(having)));
    }
    if query.order_by.is_some() {
        let terms=query.order_terms().map(|order|{
            let metric=order.column.strip_prefix("#aggregate")
                .and_then(|name|name.parse::<usize>().ok());
            (metric,order.descending,order.nulls_first)
        }).collect::<Vec<_>>();
        groups.sort_by(|left,right|{
            for &(metric,descending,nulls_first) in &terms {
                let order=if let Some(index)=metric {
                    typed::compare_order_cells(&left.1[index].sort_cell(),
                        &right.1[index].sort_cell(),descending,nulls_first)
                } else {
                    let cell1=typed::Cell::Numeric(Number::Unsigned(left.0));
                    let cell2=typed::Cell::Numeric(Number::Unsigned(right.0));
                    typed::compare_order_cells(&cell1,&cell2,descending,nulls_first)
                };
                if order!=Ordering::Equal {return order;}
            }
            left.0.cmp(&right.0)
        });
    }
    for (value,states) in groups.iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX)) {
        write!(output,"{value}")?;
        for state in states {
            write!(output,"\t{}",state.render())?;
        }
        writeln!(output)?;
    }
    Ok(())
}

#[cfg(test)]
mod generated_multi_group_having_tests {
    use crate::{execute, parse_select};

    fn run(sql: &str) -> String {
        let mut bytes = Vec::new();
        execute(&parse_select(sql).unwrap(), None, &mut bytes).unwrap();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn filters_generated_group_metrics_before_paging() {
        assert_eq!(run(
            "SELECT number, sum(number), max(number) FROM numbers(5) \
             GROUP BY number HAVING sum(number) >= 3 ORDER BY number LIMIT 1 OFFSET 1"
        ), "number\tsum(number)\tmax(number)\n4\t4\t4\n");
    }

    #[test]
    fn billion_row_grouped_multi_metric_page_is_direct() {
        assert_eq!(run(
            "SELECT number, sum(number), max(number) FROM numbers(1000000000) \
             GROUP BY number LIMIT 2 OFFSET 999999998"
        ), "number\tsum(number)\tmax(number)\n999999998\t999999998\t999999998\n999999999\t999999999\t999999999\n");
        assert_eq!(run(
            "SELECT number, sum(number), max(number) FROM numbers(1000000000) \
             WHERE number >= 999999996 GROUP BY number LIMIT 2 OFFSET 1"
        ), "number\tsum(number)\tmax(number)\n999999997\t999999997\t999999997\n999999998\t999999998\t999999998\n");
    }

    #[test]
    fn having_does_not_return_filtered_generated_groups() {
        assert_eq!(run(
            "SELECT number, sum(number), max(number) FROM numbers(4) \
             GROUP BY number HAVING max(number) > 100"
        ), "number\tsum(number)\tmax(number)\n");
    }
}

#[cfg(test)]
mod generated_group_state_budget_tests {
    use super::*;

    #[test]
    fn budgets_wide_group_metric_vectors_before_allocating() {
        let items = (0..64).map(|_| (AggregateFunction::Sum, "number".into()))
            .collect::<Vec<_>>();
        let mut used = 0;
        charge_group_states(&mut used, &items).unwrap();
        let first = used;
        charge_group_states(&mut used, &items).unwrap();
        assert_eq!(used, first * 2);
        used = MAX_GENERATED_GROUP_BYTES - 1;
        assert!(charge_group_states(&mut used, &items).is_err());
        assert_eq!(used, MAX_GENERATED_GROUP_BYTES - 1);
    }
}
