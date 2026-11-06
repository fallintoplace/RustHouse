//! Combine timestamp groups by minute of hour, retaining the other keys.
use super::*;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_GROUPS: usize = 200_000;
const MAX_GROUP_BYTES: usize = 64 * 1024 * 1024;
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(data.len()) > MAX_BYTES {
            return Err(io::Error::other(
                "EXTRACT grouping intermediate exceeds 16 MiB",
            ));
        }
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// The group key order resolves ties, so selecting the requested prefix
// before sorting gives the same output without sorting all input groups.
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

fn minute_group_key(raw_time: &[u8]) -> Result<Vec<u8>, ExecutionError> {
    if raw_time == b"\\N" {
        return Ok(b"\\N".to_vec());
    }
    let timestamp = std::str::from_utf8(raw_time)
        .map_err(|_| ExecutionError::Invalid("invalid EXTRACT timestamp".into()))?;
    let seconds = rusthouse_storage::parse_datetime(timestamp)
        .ok_or_else(|| ExecutionError::Invalid("invalid EXTRACT timestamp".into()))?;
    Ok(((seconds / 60) % 60).to_string().into_bytes())
}

pub(super) fn execute<W: Write>(
    query: &Select,
    path: Option<&Path>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let extract = query
        .grouped_extract_minute
        .as_ref()
        .ok_or_else(|| ExecutionError::Invalid("EXTRACT grouping plan is missing".into()))?;
    let path = path.ok_or_else(|| {
        ExecutionError::Invalid("EXTRACT grouping requires a stored table".into())
    })?;
    let metadata = inspect_query_source(path)?;
    if metadata
        .columns
        .iter()
        .any(|column| column.name.eq_ignore_ascii_case(&extract.label))
    {
        return Err(ExecutionError::Invalid(
            "EXTRACT alias conflicts with a source column".into(),
        ));
    }
    let keys = match &query.projection {
        Projection::GroupedMultiCount { columns }
        | Projection::GroupedMultiAggregates { columns, .. } => columns,
        _ => {
            return Err(ExecutionError::Invalid(
                "EXTRACT grouping requires three keys".into(),
            ));
        }
    };
    if keys.len() != 3 {
        return Err(ExecutionError::Invalid(
            "EXTRACT grouping requires three keys".into(),
        ));
    }
    for name in [&keys[0], &keys[2]] {
        let kind = metadata
            .columns
            .iter()
            .find(|column| &column.name == name)
            .ok_or_else(|| ExecutionError::Invalid("unknown EXTRACT group key".into()))?;
        if matches!(typed::base_type(&kind.data_type), "Float32" | "Float64") {
            return Err(ExecutionError::Invalid(
                "EXTRACT grouping does not support floating point keys".into(),
            ));
        }
    }
    let column = metadata
        .columns
        .iter()
        .find(|column| column.name == extract.column)
        .ok_or_else(|| ExecutionError::Invalid("unknown EXTRACT grouping column".into()))?;
    if typed::base_type(&column.data_type) != "DateTime" {
        return Err(ExecutionError::Invalid(
            "EXTRACT minute requires DateTime".into(),
        ));
    }
    let mut source = query.clone();
    source.grouped_extract_minute = None;
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
        .ok_or_else(|| ExecutionError::Invalid("missing EXTRACT grouping header".into()))?
        .split(|&byte| byte == b'\t')
        .collect::<Vec<_>>();
    if header.len() != 4 {
        return Err(ExecutionError::Invalid(
            "invalid EXTRACT grouping header".into(),
        ));
    }
    // Fixed-size keys and direct field splitting avoid two temporary
    // allocations per row before hash probing.
    let mut groups = HashMap::<[Vec<u8>; 3], u64>::new();
    let mut retained = 0usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split(|&byte| byte == b'\t');
        let (Some(first), Some(raw_time), Some(last), Some(raw_count)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(ExecutionError::Invalid(
                "invalid EXTRACT grouping row".into(),
            ));
        };
        if fields.next().is_some() {
            return Err(ExecutionError::Invalid(
                "invalid EXTRACT grouping row".into(),
            ));
        }
        let minute = minute_group_key(raw_time)?;
        let key = [first.to_vec(), minute, last.to_vec()];
        let count = std::str::from_utf8(raw_count)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| ExecutionError::Invalid("invalid EXTRACT grouping count".into()))?;
        let group_count = groups.len();
        let group = match groups.entry(key) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                if group_count >= MAX_GROUPS {
                    return Err(ExecutionError::Invalid(
                        "EXTRACT grouping exceeds 200000 groups".into(),
                    ));
                }
                let bytes = entry
                    .key()
                    .iter()
                    .try_fold(128usize, |sum, value| {
                        sum.checked_add(value.len().saturating_mul(2))
                    })
                    .ok_or_else(|| {
                        ExecutionError::Invalid("EXTRACT group memory overflow".into())
                    })?;
                retained = retained
                    .checked_add(bytes)
                    .filter(|&n| n <= MAX_GROUP_BYTES)
                    .ok_or_else(|| {
                        ExecutionError::Invalid("EXTRACT groups exceed 64 MiB".into())
                    })?;
                entry.insert(0)
            }
        };
        *group = group
            .checked_add(count)
            .ok_or_else(|| ExecutionError::Invalid("EXTRACT grouping count overflow".into()))?;
    }
    let mut rows = groups.into_iter().collect::<Vec<_>>();
    let descending = query
        .order_by
        .as_ref()
        .is_some_and(|order| order.descending);
    sort_group_page(
        &mut rows,
        descending,
        query.offset.unwrap_or(0),
        query.limit,
    );
    output.write_all(header[0])?;
    write!(output, "\t{}\t", extract.label)?;
    output.write_all(header[2])?;
    output.write_all(b"\t")?;
    output.write_all(header[3])?;
    output.write_all(b"\n")?;
    for (keys, count) in rows
        .into_iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        output.write_all(&keys[0])?;
        output.write_all(b"\t")?;
        output.write_all(&keys[1])?;
        output.write_all(b"\t")?;
        output.write_all(&keys[2])?;
        writeln!(output, "\t{count}")?;
    }
    Ok(())
}

#[cfg(test)]
mod topk_tests {
    use super::sort_group_page;

    #[test]
    fn bounded_extract_group_pages_match_full_sort_with_ties() {
        let groups = (0..128u32)
            .map(|key| (key, u64::from(key % 7)))
            .collect::<Vec<_>>();
        for descending in [false, true] {
            for (offset, limit) in [
                (0, Some(1)),
                (3, Some(20)),
                (120, Some(30)),
                (128, None),
                (0, None),
                (0, Some(0)),
            ] {
                let mut expected = groups.clone();
                expected.sort_by(|a, b| {
                    let cmp = if descending {
                        b.1.cmp(&a.1)
                    } else {
                        a.1.cmp(&b.1)
                    };
                    cmp.then_with(|| a.0.cmp(&b.0))
                });
                let mut actual = groups.clone();
                sort_group_page(&mut actual, descending, offset, limit);
                assert_eq!(
                    actual
                        .into_iter()
                        .skip(offset)
                        .take(limit.unwrap_or(usize::MAX))
                        .collect::<Vec<_>>(),
                    expected
                        .into_iter()
                        .skip(offset)
                        .take(limit.unwrap_or(usize::MAX))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}
