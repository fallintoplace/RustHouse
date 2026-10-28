//! Exact COUNT(DISTINCT) segmented by multiple grouping dimensions.
use super::typed::{self, Cell, Key};
use super::{ExecutionError, Number};
use crate::Select;
use rusthouse_storage::{TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 200_000;
const MAX_PAIRS: usize = 200_000;

struct Group {
    keys: Vec<Cell>,
    values: HashSet<Key>,
}

pub(super) fn execute<W: Write>(
    query: &Select,
    part: &Path,
    columns: &[String],
    value_column: &str,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let metadata=inspect_query_source(part)?;
    let types=metadata.columns.iter()
        .map(|column|(column.name.clone(),column.data_type.clone()))
        .collect::<HashMap<_,_>>();
    let mut names=columns.to_vec();
    if !names.iter().any(|column|column==value_column) {
        names.push(value_column.to_owned());
    }
    for predicate in query.predicates.iter().chain(query.or_predicates.iter().flatten()) {
        if !names.iter().any(|column|column==predicate.column()) {
            names.push(predicate.column().to_owned());
        }
    }
    for name in &names {
        if !types.contains_key(name) {
            return Err(ExecutionError::Invalid(format!("unknown column: {name}")));
        }
    }
    let indices=names.iter().enumerate()
        .map(|(index,name)|(name.clone(),index)).collect::<HashMap<_,_>>();
    let mut conditions=vec![typed::compile(&query.predicates,&indices,&types)?];
    for branch in &query.or_predicates {
        conditions.push(typed::compile(branch,&indices,&types)?);
    }
    writeln!(output,"{}\tcountDistinct({value_column})",columns.join("\t"))?;
    if query.limit==Some(0) { return Ok(()); }
    let group_indices=columns.iter().map(|name|indices[name]).collect::<Vec<_>>();
    let value_index=indices[value_column];
    let mut reader=TypedQueryReader::open_with_metadata(part,&names,&metadata)?;
    let mut groups=Vec::<Group>::new();
    let mut lookup=HashMap::<Vec<Key>,usize>::new();
    let mut probes=Vec::<Key>::with_capacity(columns.len());
    let mut total_pairs=0usize;
    let no_filters=query.predicates.is_empty()&&query.or_predicates.is_empty();
    while let Some(batch)=reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch,row,&conditions) { continue; }
            probes.clear();
            for &index in &group_indices {
                probes.push(typed::key_at(&batch[index],row));
            }
            let group=if let Some(&index)=lookup.get(probes.as_slice()) {
                index
            } else {
                if groups.len()>=MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "GROUP BY supports at most 200000 groups".into()));
                }
                let index=groups.len();
                lookup.insert(std::mem::take(&mut probes),index);
                probes=Vec::with_capacity(columns.len());
                groups.push(Group {
                    keys:group_indices.iter().map(|&i|typed::cell(&batch[i],row)).collect(),
                    values:HashSet::new(),
                });
                index
            };
            if typed::is_null_at(&batch[value_index],row) { continue; }
            let value=typed::key_at(&batch[value_index],row);
            if !groups[group].values.contains(&value) {
                if total_pairs>=MAX_PAIRS {
                    return Err(ExecutionError::Invalid(
                        "grouped COUNT(DISTINCT) supports at most 200000 distinct pairs".into()));
                }
                groups[group].values.insert(value);
                total_pairs+=1;
            }
        }
    }
    if let Some(having)=&query.having {
        groups.retain(|group| {
            super::predicates::number_matches(
                Number::Unsigned(group.values.len() as u128),
                Number::Unsigned(having.value as u128),
                having.operator,
            )
        });
    }
    if query.order_by.is_some() {
        let terms=query.order_terms().map(|term|{
            let index=columns.iter().position(|key|key==&term.column);
            (index,term.descending,term.nulls_first)
        }).collect::<Vec<_>>();
        groups.sort_by(|a,b|{
            for &(index,desc,nulls_first) in &terms {
                let comparison=if let Some(index)=index {
                    typed::compare_order_cells(&a.keys[index],&b.keys[index],desc,nulls_first)
                } else {
                    let order=a.values.len().cmp(&b.values.len());
                    if desc {order.reverse()} else {order}
                };
                if comparison!=Ordering::Equal {return comparison;}
            }
            Ordering::Equal
        });
    }
    let output_types=columns.iter().map(|column|types[column].as_str()).collect::<Vec<_>>();
    for group in groups.iter().skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX)) {
        for (index,cell) in group.keys.iter().enumerate() {
            if index>0 {write!(output,"\t")?;}
            typed::write_typed_value(output,cell,output_types[index])?;
        }
        writeln!(output,"\t{}",group.values.len())?;
    }
    Ok(())
}
