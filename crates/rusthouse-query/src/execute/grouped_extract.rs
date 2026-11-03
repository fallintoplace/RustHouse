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
    let mut groups = HashMap::<Vec<Vec<u8>>, u64>::new();
    let mut retained = 0usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let fields = line.split(|&byte| byte == b'\t').collect::<Vec<_>>();
        if fields.len() != 4 {
            return Err(ExecutionError::Invalid(
                "invalid EXTRACT grouping row".into(),
            ));
        }
        let minute = if fields[1] == b"\\N" {
            b"\\N".to_vec()
        } else {
            let timestamp = std::str::from_utf8(fields[1])
                .map_err(|_| ExecutionError::Invalid("invalid EXTRACT timestamp".into()))?;
            let seconds = rusthouse_storage::parse_datetime(timestamp)
                .ok_or_else(|| ExecutionError::Invalid("invalid EXTRACT timestamp".into()))?;
            ((seconds / 60) % 60).to_string().into_bytes()
        };
        let key = vec![fields[0].to_vec(), minute, fields[2].to_vec()];
        let count = std::str::from_utf8(fields[3])
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| ExecutionError::Invalid("invalid EXTRACT grouping count".into()))?;
        if !groups.contains_key(&key) {
            if groups.len() >= MAX_GROUPS {
                return Err(ExecutionError::Invalid(
                    "EXTRACT grouping exceeds 200000 groups".into(),
                ));
            }
            let bytes = key
                .iter()
                .try_fold(128usize, |sum, value| {
                    sum.checked_add(value.len().saturating_mul(2))
                })
                .ok_or_else(|| ExecutionError::Invalid("EXTRACT group memory overflow".into()))?;
            retained = retained
                .checked_add(bytes)
                .filter(|&n| n <= MAX_GROUP_BYTES)
                .ok_or_else(|| ExecutionError::Invalid("EXTRACT groups exceed 64 MiB".into()))?;
        }
        let group = groups.entry(key).or_insert(0);
        *group = group
            .checked_add(count)
            .ok_or_else(|| ExecutionError::Invalid("EXTRACT grouping count overflow".into()))?;
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
