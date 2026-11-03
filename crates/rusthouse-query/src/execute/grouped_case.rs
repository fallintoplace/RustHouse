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
// Group keys are unique, so a bounded result needs only the prefix ending
// at OFFSET + LIMIT rather than a full sort of every group.
fn sort_group_page<K: Ord>(
    rows: &mut Vec<(K, u64)>,
    descending: bool,
    offset: usize,
    limit: Option<usize>,
) {
    if limit == Some(0) || offset >= rows.len() {
        rows.clear();
        return;
    }
    let compare = |a: &(K, u64), b: &(K, u64)| {
        let cmp = if descending {
            b.1.cmp(&a.1)
        } else {
            a.1.cmp(&b.1)
        };
        cmp.then_with(|| a.0.cmp(&b.0))
    };
    let page_end = offset
        .saturating_add(limit.unwrap_or(usize::MAX))
        .min(rows.len());
    if page_end < rows.len() {
        rows.select_nth_unstable_by(page_end - 1, &compare);
        rows.truncate(page_end);
    }
    rows.sort_unstable_by(compare);
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
    let mut groups = HashMap::<Vec<Vec<u8>>, u64>::new();
    let mut retained = 0usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let fields = line.split(|&byte| byte == b'\t').collect::<Vec<_>>();
        if fields.len() != 6 {
            return Err(ExecutionError::Invalid("invalid CASE grouping row".into()));
        }
        let case_value = if fields[1] == b"0" && fields[2] == b"0" {
            fields[3]
        } else {
            b""
        };
        let key = vec![
            fields[0].to_vec(),
            fields[1].to_vec(),
            fields[2].to_vec(),
            case_value.to_vec(),
            fields[4].to_vec(),
        ];
        let count = std::str::from_utf8(fields[5])
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| ExecutionError::Invalid("invalid CASE grouping count".into()))?;
        if !groups.contains_key(&key) {
            if groups.len() >= MAX_GROUPS {
                return Err(ExecutionError::Invalid(
                    "CASE grouping exceeds 200000 groups".into(),
                ));
            }
            let bytes = key
                .iter()
                .try_fold(192usize, |sum, value| {
                    sum.checked_add(value.len().saturating_mul(2))
                })
                .ok_or_else(|| ExecutionError::Invalid("CASE group memory overflow".into()))?;
            retained = retained
                .checked_add(bytes)
                .filter(|&n| n <= MAX_GROUP_BYTES)
                .ok_or_else(|| ExecutionError::Invalid("CASE groups exceed 64 MiB".into()))?;
        }
        let group = groups.entry(key).or_insert(0);
        *group = group
            .checked_add(count)
            .ok_or_else(|| ExecutionError::Invalid("CASE grouping count overflow".into()))?;
    }
    let mut rows = groups.into_iter().collect::<Vec<_>>();
    let descending = query
        .order_by
        .as_ref()
        .is_some_and(|order| order.descending);
    sort_group_page(&mut rows, descending, query.offset.unwrap_or(0), query.limit);
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

#[cfg(test)]
mod topk_tests {
    use super::sort_group_page;

    #[test]
    fn bounded_case_group_pages_match_full_sort_with_ties() {
        let groups = (0..128u32)
            .map(|key| (key, u64::from(key % 9)))
            .collect::<Vec<_>>();
        for descending in [false, true] {
            for (offset, limit) in [(0, Some(1)), (5, Some(11)), (120, Some(20)), (128, None), (0, None), (0, Some(0))] {
                let mut expected = groups.clone();
                expected.sort_by(|a, b| {
                    let cmp = if descending { b.1.cmp(&a.1) } else { a.1.cmp(&b.1) };
                    cmp.then_with(|| a.0.cmp(&b.0))
                });
                let mut actual = groups.clone();
                sort_group_page(&mut actual, descending, offset, limit);
                assert_eq!(
                    actual.into_iter().skip(offset).take(limit.unwrap_or(usize::MAX)).collect::<Vec<_>>(),
                    expected.into_iter().skip(offset).take(limit.unwrap_or(usize::MAX)).collect::<Vec<_>>()
                );
            }
        }
    }
}
