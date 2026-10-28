//! Fused group metrics over generated numeric series, without stored parts.
use super::{AggregateState, ExecutionError, Number, compile_numbers_predicates, number_matches_all, typed};
use crate::{AggregateFunction, Select};
use std::cmp::Ordering;
use std::io::Write;

const MAX_GROUPS: usize = 200_000;

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
    let mut groups=Vec::<(u128,Vec<AggregateState>)>::new();
    for index in 0..rows {
        let value=u128::from(start)+u128::from(index)*u128::from(step);
        if !number_matches_all(Number::Unsigned(value),&comparisons) {continue;}
        if groups.len()>=MAX_GROUPS {
            return Err(ExecutionError::Invalid(
                "generated GROUP BY supports at most 200000 groups".into()));
        }
        // The virtual number column is unique for nonzero steps. Every row
        // forms one group and can share the regular aggregate-state semantics.
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
