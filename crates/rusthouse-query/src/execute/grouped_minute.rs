//! Combine bounded timestamp counts into UTC minute counts before pagination.
use super::*;

const MAX_INTERMEDIATE_BYTES: usize = 16 * 1024 * 1024;
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(data.len()) > MAX_INTERMEDIATE_BYTES {
            return Err(io::Error::other(
                "minute grouping intermediate exceeds 16 MiB",
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
    let minute = query
        .grouped_minute
        .as_ref()
        .ok_or_else(|| ExecutionError::Invalid("minute plan is missing".into()))?;
    let path = path
        .ok_or_else(|| ExecutionError::Invalid("minute grouping requires a stored table".into()))?;
    let metadata = inspect_query_source(path)?;
    let column = metadata
        .columns
        .iter()
        .find(|column| column.name == minute.column)
        .ok_or_else(|| ExecutionError::Invalid("unknown minute grouping column".into()))?;
    if typed::base_type(&column.data_type) != "DateTime" {
        return Err(ExecutionError::Invalid(
            "minute grouping requires DateTime".into(),
        ));
    }
    let mut source = query.clone();
    source.grouped_minute = None;
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
    let text = std::str::from_utf8(&intermediate.0)
        .map_err(|_| ExecutionError::Invalid("invalid minute grouping output".into()))?;
    let mut lines = text.lines();
    let header = lines
        .next()
        .and_then(|line| line.split_once('\t'))
        .ok_or_else(|| ExecutionError::Invalid("invalid minute grouping header".into()))?;
    let mut groups: HashMap<Option<u32>, u64> = HashMap::new();
    for line in lines {
        let (value, count) = line
            .split_once('\t')
            .ok_or_else(|| ExecutionError::Invalid("invalid minute grouping row".into()))?;
        let minute = if value == r"\N" {
            None
        } else {
            let seconds = rusthouse_storage::parse_datetime(value)
                .ok_or_else(|| ExecutionError::Invalid("invalid DateTime group".into()))?;
            Some(seconds / 60 * 60)
        };
        let count = count
            .parse::<u64>()
            .map_err(|_| ExecutionError::Invalid("invalid minute grouping count".into()))?;
        let group = groups.entry(minute).or_insert(0);
        *group = group
            .checked_add(count)
            .ok_or_else(|| ExecutionError::Invalid("minute grouping count overflow".into()))?;
    }
    let mut rows = groups.into_iter().collect::<Vec<_>>();
    let (descending, nulls_first) = query
        .order_by
        .as_ref()
        .map_or((false, None), |order| (order.descending, order.nulls_first));
    let cell = |value: Option<u32>| {
        value.map_or(typed::Cell::Null, |n| {
            typed::Cell::Numeric(Number::Unsigned(u128::from(n)))
        })
    };
    // Only order the prefix needed by LIMIT/OFFSET. Sorting every group
    // is unnecessary for a bounded page over a large input.
    let compare = |a: &(Option<u32>, u64), b: &(Option<u32>, u64)| {
        typed::compare_order_cells(&cell(a.0), &cell(b.0), descending, nulls_first)
    };
    let page_end = query
        .offset
        .unwrap_or(0)
        .saturating_add(query.limit.unwrap_or(usize::MAX))
        .min(rows.len());
    if page_end < rows.len() {
        if page_end == 0 {
            rows.clear();
        } else {
            rows.select_nth_unstable_by(page_end - 1, &compare);
            rows.truncate(page_end);
        }
    }
    rows.sort_by(compare);
    writeln!(output, "{}\t{}", minute.label, header.1)?;
    for (value, count) in rows
        .into_iter()
        .skip(query.offset.unwrap_or(0))
        .take(query.limit.unwrap_or(usize::MAX))
    {
        if let Some(value) = value {
            write!(output, "{}", rusthouse_storage::format_datetime(value))?;
        } else {
            write!(output, "\\N")?;
        }
        writeln!(output, "\t{count}")?;
    }
    Ok(())
}
