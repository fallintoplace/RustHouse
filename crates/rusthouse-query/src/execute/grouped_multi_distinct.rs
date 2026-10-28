//! Exact COUNT(DISTINCT) segmented by multiple grouping dimensions.
use super::typed::{self, Cell, Key};
use super::{ExecutionError, Number};
use crate::Select;
use rusthouse_storage::{NumericColumn, TypedColumn, TypedQueryReader, inspect_query_source};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

const MAX_GROUPS: usize = 200_000;
const MAX_PAIRS: usize = 200_000;

struct Group {
    keys: Vec<Cell>,
    values: HashSet<Key>,
    text_values: HashSet<Vec<u8>>,
    uuid_values: HashSet<[u8; 16]>,
    // Exact cardinality of a byte-wide column needs only 256 bits per group.
    byte_values: [u64; 4],
}

impl Group {
    fn cardinality(&self) -> usize {
        self.values.len() + self.text_values.len() + self.uuid_values.len()
            + self.byte_values.iter().map(|word| word.count_ones() as usize).sum::<usize>()
    }
}

fn numeric_byte_at(column: &TypedColumn, row: usize) -> Option<u8> {
    let values = match column {
        TypedColumn::Numeric(values) | TypedColumn::NullableNumeric { values, .. } => values,
        _ => return None,
    };
    match values {
        NumericColumn::UInt8(values) => Some(values[row]),
        NumericColumn::Int8(values) => Some(values[row] as u8),
        _ => None,
    }
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
    let byte_values=matches!(typed::base_type(types[value_column].as_str()), "UInt8" | "Int8");
    let mut reader=TypedQueryReader::open_with_metadata(part,&names,&metadata)?;
    let mut groups=Vec::<Group>::new();
    let mut lookup=HashMap::<Vec<Key>,usize>::new();
    let has_text_key = columns.iter().any(|column| matches!(
        typed::base_type(types[column].as_str()), "String"
    ));
    let mut encoded_lookup = HashMap::<Vec<u8>,usize>::new();
    let mut encoded_probe = Vec::<u8>::new();
    let mut probes=Vec::<Key>::with_capacity(columns.len());
    let mut total_pairs=0usize;
    let no_filters=query.predicates.is_empty()&&query.or_predicates.is_empty();
    while let Some(batch)=reader.next_batch()? {
        for row in 0..batch[0].len() {
            if !no_filters && !typed::passes(&batch,row,&conditions) { continue; }
            let group = if has_text_key {
                encoded_probe.clear();
                for &index in &group_indices {
                    super::grouped_numeric_keys::append_group_component(
                        &batch[index], row, &mut encoded_probe
                    );
                }
                if let Some(&index) = encoded_lookup.get(encoded_probe.as_slice()) {
                    index
                } else {
                    if groups.len() >= MAX_GROUPS {
                        return Err(ExecutionError::Invalid(
                            "GROUP BY supports at most 200000 groups".into()
                        ));
                    }
                    let index = groups.len();
                    encoded_lookup.insert(std::mem::take(&mut encoded_probe), index);
                    encoded_probe = Vec::with_capacity(columns.len() * 24);
                    groups.push(Group {
                        keys: group_indices.iter().map(|&i|typed::cell(&batch[i],row)).collect(),
                        values: HashSet::new(),
                        text_values: HashSet::new(),
                        uuid_values: HashSet::new(),
                        byte_values: [0; 4],
                    });
                    index
                }
            } else {
                probes.clear();
                for &index in &group_indices {
                    probes.push(typed::key_at(&batch[index],row));
                }
                if let Some(&index) = lookup.get(probes.as_slice()) {
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
                        text_values: HashSet::new(),
                        uuid_values: HashSet::new(),
                        byte_values: [0; 4],
                    });
                    index
                }
            };
            if typed::is_null_at(&batch[value_index],row) { continue; }
            if byte_values {
                let byte = numeric_byte_at(&batch[value_index], row)
                    .ok_or_else(|| ExecutionError::Invalid("expected byte-width DISTINCT value".into()))?;
                groups[group].byte_values[usize::from(byte) / 64] |=
                    1u64 << (usize::from(byte) % 64);
                continue;
            }
            let text_value = match &batch[value_index] {
                TypedColumn::Text(values) => values.get(row),
                TypedColumn::NullableText { values, nulls } if nulls[row] == 0 => values.get(row),
                _ => None,
            };
            if let Some(bytes) = text_value {
                if !groups[group].text_values.contains(bytes) {
                    if total_pairs >= MAX_PAIRS {
                        return Err(ExecutionError::Invalid(
                            "grouped COUNT(DISTINCT) supports at most 200000 distinct pairs".into()
                        ));
                    }
                    groups[group].text_values.insert(bytes.to_vec());
                    total_pairs += 1;
                }
                continue;
            }
            if let TypedColumn::Uuid(values) = &batch[value_index] {
                let bytes = *values[row].as_bytes();
                if !groups[group].uuid_values.contains(&bytes) {
                    if total_pairs >= MAX_PAIRS {
                        return Err(ExecutionError::Invalid(
                            "grouped COUNT(DISTINCT) supports at most 200000 distinct pairs".into()
                        ));
                    }
                    groups[group].uuid_values.insert(bytes);
                    total_pairs += 1;
                }
                continue;
            }
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
                Number::Unsigned(group.cardinality() as u128),
                Number::Unsigned(having.value as u128),
                having.operator,
            )
        });
    }
    let mut bounded_order = None;
    if query.order_by.is_some() {
        let terms=query.order_terms().map(|term|{
            let index=columns.iter().position(|key|key==&term.column);
            (index,term.descending,term.nulls_first)
        }).collect::<Vec<_>>();
        let compare = |a: &Group, b: &Group| {
            for &(index,desc,nulls_first) in &terms {
                let comparison=if let Some(index)=index {
                    typed::compare_order_cells(&a.keys[index],&b.keys[index],desc,nulls_first)
                } else {
                    let order=a.cardinality().cmp(&b.cardinality());
                    if desc {order.reverse()} else {order}
                };
                if comparison!=Ordering::Equal {return comparison;}
            }
            Ordering::Equal
        };
        let end = query.limit
            .and_then(|limit| query.offset.unwrap_or(0).checked_add(limit));
        if let Some(end) = end.filter(|&end| end < groups.len()) {
            let mut indices = (0..groups.len()).collect::<Vec<_>>();
            let compare_index = |left: &usize, right: &usize| {
                compare(&groups[*left], &groups[*right]).then_with(|| left.cmp(right))
            };
            indices.select_nth_unstable_by(end, &compare_index);
            indices.truncate(end);
            indices.sort_unstable_by(compare_index);
            bounded_order = Some(indices);
        } else {
            groups.sort_by(compare);
        }
    }
    let output_types=columns.iter().map(|column|types[column].as_str()).collect::<Vec<_>>();
    let write_group = |group: &Group, output: &mut W| -> Result<(), ExecutionError> {
        for (index,cell) in group.keys.iter().enumerate() {
            if index>0 {write!(output,"\t")?;}
            typed::write_typed_value(output,cell,output_types[index])?;
        }
        writeln!(output,"\t{}",group.cardinality())?;
        Ok(())
    };
    if let Some(indices) = bounded_order {
        for index in indices.into_iter().skip(query.offset.unwrap_or(0)) {
            write_group(&groups[index], output)?;
        }
    } else {
        for group in groups.iter().skip(query.offset.unwrap_or(0))
            .take(query.limit.unwrap_or(usize::MAX)) {
            write_group(group, output)?;
        }
    }
    Ok(())
}
