//! Combine raw string groups into a bounded conditional CASE grouping.
use super::*;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_GROUPS: usize = 200_000;
const MAX_GROUP_BYTES: usize = 64 * 1024 * 1024;
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(data.len()) > MAX_BYTES {
            return Err(io::Error::other(
                "CASE grouping intermediate exceeds 16 MiB",
            ));
        }
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn integer(kind: &str) -> bool {
    matches!(
        typed::base_type(kind),
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
pub(super) fn execute<W: Write>(
    query: &Select,
    path: Option<&Path>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let case = query
        .grouped_case_key
        .as_ref()
        .ok_or_else(|| ExecutionError::Invalid("CASE grouping plan is missing".into()))?;
    let path = path
        .ok_or_else(|| ExecutionError::Invalid("CASE grouping requires a stored table".into()))?;
    let metadata = inspect_query_source(path)?;
    if metadata.columns.iter().any(|column| {
        column.name.eq_ignore_ascii_case(&case.case_label)
            || column.name.eq_ignore_ascii_case(&case.destination_label)
    }) {
        return Err(ExecutionError::Invalid(
            "CASE alias conflicts with a source column".into(),
        ));
    }
    let keys = match &query.projection {
        Projection::GroupedMultiCount { columns }
        | Projection::GroupedMultiAggregates { columns, .. } => columns,
        _ => {
            return Err(ExecutionError::Invalid(
                "CASE grouping requires five keys".into(),
            ));
        }
    };
    if keys.len() != 5 {
        return Err(ExecutionError::Invalid(
            "CASE grouping requires five keys".into(),
        ));
    }
    for (index, name) in keys.iter().enumerate() {
        let kind = metadata
            .columns
            .iter()
            .find(|column| &column.name == name)
            .ok_or_else(|| ExecutionError::Invalid("unknown CASE group key".into()))?;
        if index < 3 && !integer(&kind.data_type) {
            return Err(ExecutionError::Invalid(
                "CASE grouping requires integer keys".into(),
            ));
        }
        if index >= 3 && typed::base_type(&kind.data_type) != "String" {
            return Err(ExecutionError::Invalid(
                "CASE grouping requires String keys".into(),
            ));
        }
    }
    if keys[1] != case.first_flag || keys[2] != case.second_flag {
        return Err(ExecutionError::Invalid("CASE grouping flags differ".into()));
    }
    let mut source = query.clone();
    source.grouped_case_key = None;
    source.order_by = None;
    source.order_by_extra.clear();
    source.offset = None;
    source.limit = if query.limit == Some(0) {
        Some(0)
    } else {
        None
    };
    let mut intermediate = Bounded(Vec::new());
    execute_into(&source, Some(path), &mut intermediate)?;
    let mut lines = intermediate.0.split(|&byte| byte == b'\n');
    let header = lines
        .next()
        .ok_or_else(|| ExecutionError::Invalid("missing CASE grouping header".into()))?
        .split(|&byte| byte == b'\t')
        .collect::<Vec<_>>();
    if header.len() != 6 {
        return Err(ExecutionError::Invalid(
            "invalid CASE grouping header".into(),
        ));
    }
    // A fixed-size key removes the outer Vec allocation on each input row.
    let mut groups = HashMap::<[Vec<u8>; 5], u64>::new();
    let mut retained = 0usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split(|&byte| byte == b'\t');
        let (Some(first), Some(flag1), Some(flag2), Some(destination), Some(last), Some(raw_count)) =
            (
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
            )
        else {
            return Err(ExecutionError::Invalid("invalid CASE grouping row".into()));
        };
        if fields.next().is_some() {
            return Err(ExecutionError::Invalid("invalid CASE grouping row".into()));
        }
        let case_value = if flag1 == b"0" && flag2 == b"0" {
            destination
        } else {
            b""
        };
        let key = [
            first.to_vec(),
            flag1.to_vec(),
            flag2.to_vec(),
            case_value.to_vec(),
            last.to_vec(),
        ];
        let count = std::str::from_utf8(raw_count)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| ExecutionError::Invalid("invalid CASE grouping count".into()))?;
        let group_count = groups.len();
        let group = match groups.entry(key) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                if group_count >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "CASE grouping exceeds 200000 groups".into(),
                    ));
                }
                let bytes = entry
                    .key()
                    .iter()
                    .try_fold(192usize, |sum, value| {
                        sum.checked_add(value.len().saturating_mul(2))
                    })
                    .ok_or_else(|| ExecutionError::Invalid("CASE group memory overflow".into()))?;
                retained = retained
                    .checked_add(bytes)
                    .filter(|&n| n <= MAX_GROUP_BYTES)
                    .ok_or_else(|| ExecutionError::Invalid("CASE groups exceed 64 MiB".into()))?;
                entry.insert(0)
            }
        };
        *group = group
            .checked_add(count)
            .ok_or_else(|| ExecutionError::Invalid("CASE grouping count overflow".into()))?;
    }
    let mut rows = groups.into_iter().collect::<Vec<_>>();
    let descending = query
        .order_by
        .as_ref()
        .is_some_and(|order| order.descending);
    rows.sort_by(|a, b| {
        let cmp = if descending {
            b.1.cmp(&a.1)
        } else {
            a.1.cmp(&b.1)
        };
        cmp.then_with(|| a.0.cmp(&b.0))
    });
    output.write_all(header[0])?;
    output.write_all(b"\t")?;
    output.write_all(header[1])?;
    output.write_all(b"\t")?;
    output.write_all(header[2])?;
    write!(
        output,
        "\t{}\t{}\t",
        case.case_label, case.destination_label
    )?;
    output.write_all(header[5])?;
    output.write_all(b"\n")?;
    for (keys, count) in rows
        .into_iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                output.write_all(b"\t")?;
            }
            output.write_all(key)?;
        }
        writeln!(output, "\t{count}")?;
    }
    Ok(())
}
