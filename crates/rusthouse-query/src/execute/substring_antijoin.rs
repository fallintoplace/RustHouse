//! Bounded execution for grouped substring queries with an average and anti-join.
use super::*;
use crate::{JoinExistsKey, Operator, SubstringAntiJoin};
use rusthouse_storage::TypedQueryReader;

fn invalid(message: &str) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}
fn key(value: &typed::Cell) -> Result<Option<JoinExistsKey>, ExecutionError> {
    match value {
        typed::Cell::Null => Ok(None),
        typed::Cell::Numeric(Number::Unsigned(v)) => Ok(Some(JoinExistsKey::Unsigned(*v))),
        typed::Cell::Numeric(Number::Signed(v)) => Ok(Some(JoinExistsKey::Signed(*v))),
        _ => Err(invalid("anti-join requires integer keys")),
    }
}
fn prefix(
    value: &typed::Cell,
    start: usize,
    length: usize,
) -> Result<Option<String>, ExecutionError> {
    match value {
        typed::Cell::Null => Ok(None),
        typed::Cell::Text(bytes) => {
            let s = std::str::from_utf8(bytes).map_err(|_| invalid("substring requires UTF-8"))?;
            Ok(Some(s.chars().skip(start - 1).take(length).collect()))
        }
        _ => Err(invalid("substring requires a String column")),
    }
}
fn integer(kind: &str) -> bool {
    matches!(
        kind,
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
    )
}
fn numeric(kind: &str) -> bool {
    integer(kind) || matches!(kind, "Float32" | "Float64")
}

pub fn execute_substring_antijoin<W: Write>(
    plan: &SubstringAntiJoin,
    table_path: &Path,
    lookup_path: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let meta = inspect_query_source(table_path)?;
    let lookup_meta = inspect_query_source(lookup_path)?;
    let find = |name: &str| {
        meta.columns
            .iter()
            .position(|c| c.name == name)
            .ok_or_else(|| invalid("unknown source column"))
    };
    let phone = find(&plan.phone)?;
    let amount = find(&plan.amount)?;
    let customer_key = find(&plan.customer_key)?;
    let lookup_key = lookup_meta
        .columns
        .iter()
        .position(|c| c.name == plan.lookup_key)
        .ok_or_else(|| invalid("unknown anti-join key"))?;
    let amount_type = typed::base_type(&meta.columns[amount].data_type);
    let key_type = typed::base_type(&meta.columns[customer_key].data_type);
    if typed::base_type(&meta.columns[phone].data_type) != "String"
        || !numeric(amount_type)
        || !integer(key_type)
        || key_type != typed::base_type(&lookup_meta.columns[lookup_key].data_type)
    {
        return Err(invalid(
            "substring anti-join requires String, numeric amount, and matching integer keys",
        ));
    }
    let threshold = typed::parse_literal(&meta.columns[amount].data_type, &plan.average_threshold)?;
    if plan.start == 0 || plan.length == 0 || plan.length > 64 {
        return Err(invalid("invalid substring bounds"));
    }
    if plan.limit == Some(0) {
        writeln!(
            output,
            "{}\t{}\t{}",
            plan.group_name, plan.count_name, plan.sum_name
        )?;
        return Ok(());
    }
    let selected: HashSet<&str> = plan.selected_prefixes.iter().map(String::as_str).collect();
    let average: HashSet<&str> = plan.average_prefixes.iter().map(String::as_str).collect();
    let fields = vec![plan.phone.clone(), plan.amount.clone()];
    let mut reader = TypedQueryReader::open(table_path, &fields)?;
    let mut sum = None;
    let mut count = 0u64;
    let mut rows = 0u64;
    while let Some(batch) = reader.next_batch()? {
        for i in 0..batch[0].len() {
            rows = rows
                .checked_add(1)
                .ok_or_else(|| invalid("row count overflow"))?;
            let Some(code) = prefix(&typed::cell(&batch[0], i), plan.start, plan.length)? else {
                continue;
            };
            let value = typed::cell(&batch[1], i);
            if average.contains(code.as_str())
                && typed::matches_comparison(&value, &threshold, Operator::Gt)
            {
                if let typed::Cell::Numeric(n) = value {
                    sum = Some(match sum {
                        Some(prev) => checked_aggregate_sum(prev, n)?,
                        None => n,
                    });
                    count = count
                        .checked_add(1)
                        .ok_or_else(|| invalid("average count overflow"))?;
                }
            }
        }
    }
    if rows != meta.rows {
        return Err(invalid("source row count changed during scan"));
    }
    let Some(sum) = sum else {
        writeln!(
            output,
            "{}\t{}\t{}",
            plan.group_name, plan.count_name, plan.sum_name
        )?;
        return Ok(());
    };
    let total = match sum {
        Number::Unsigned(v) => v as f64,
        Number::Signed(v) => v as f64,
        Number::Float(v) => v,
    };
    let avg = total / count as f64;
    if !avg.is_finite() {
        return Err(invalid("average is not finite"));
    }
    let mut keys = HashSet::new();
    let mut reader = TypedQueryReader::open(lookup_path, std::slice::from_ref(&plan.lookup_key))?;
    rows = 0;
    while let Some(batch) = reader.next_batch()? {
        for i in 0..batch[0].len() {
            rows = rows
                .checked_add(1)
                .ok_or_else(|| invalid("row count overflow"))?;
            if let Some(k) = key(&typed::cell(&batch[0], i))? {
                if !keys.contains(&k) && keys.len() >= 262_144 {
                    return Err(invalid("anti-join supports at most 262144 distinct keys"));
                }
                keys.insert(k);
            }
        }
    }
    if rows != lookup_meta.rows {
        return Err(invalid("anti-join row count changed during scan"));
    }
    let mut indices = vec![phone, amount, customer_key];
    indices.sort_unstable();
    indices.dedup();
    let fields = indices
        .iter()
        .map(|&i| meta.columns[i].name.clone())
        .collect::<Vec<_>>();
    let position = |index| indices.binary_search(&index).expect("projected column");
    let mut reader = TypedQueryReader::open(table_path, &fields)?;
    let mut groups: HashMap<String, (u64, Number)> = HashMap::new();
    rows = 0;
    while let Some(batch) = reader.next_batch()? {
        for i in 0..batch[0].len() {
            rows = rows
                .checked_add(1)
                .ok_or_else(|| invalid("row count overflow"))?;
            let Some(code) = prefix(
                &typed::cell(&batch[position(phone)], i),
                plan.start,
                plan.length,
            )?
            else {
                continue;
            };
            if !selected.contains(code.as_str()) {
                continue;
            }
            let value = typed::cell(&batch[position(amount)], i);
            if !typed::matches_comparison(
                &value,
                &typed::Cell::Numeric(Number::Float(avg)),
                Operator::Gt,
            ) {
                continue;
            }
            if key(&typed::cell(&batch[position(customer_key)], i))?
                .is_some_and(|k| keys.contains(&k))
            {
                continue;
            }
            if let typed::Cell::Numeric(n) = value {
                if let Some((count, sum)) = groups.get_mut(&code) {
                    *count = count
                        .checked_add(1)
                        .ok_or_else(|| invalid("group count overflow"))?;
                    *sum = checked_aggregate_sum(*sum, n)?;
                } else {
                    groups.insert(code, (1, n));
                }
            }
        }
    }
    if rows != meta.rows {
        return Err(invalid("source row count changed during scan"));
    }
    let mut groups = groups.into_iter().collect::<Vec<_>>();
    groups.sort_by(|a, b| {
        if plan.descending {
            b.0.cmp(&a.0)
        } else {
            a.0.cmp(&b.0)
        }
    });
    writeln!(
        output,
        "{}\t{}\t{}",
        plan.group_name, plan.count_name, plan.sum_name
    )?;
    for (code, (count, sum)) in groups
        .into_iter()
        .skip(plan.offset)
        .take(plan.limit.unwrap_or(usize::MAX))
    {
        typed::write_value(output, &typed::Cell::Text(code.into_bytes()))?;
        write!(output, "\t{count}\t")?;
        typed::write_value(output, &typed::Cell::Numeric(sum))?;
        writeln!(output)?;
    }
    Ok(())
}
