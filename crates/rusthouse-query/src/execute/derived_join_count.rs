//! Bounded second grouping over a grouped LEFT JOIN result.
use super::{ExecutionError, execute_join};
use crate::DerivedJoinCount;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;

const MAX_INTERMEDIATE_BYTES: usize = 16 * 1024 * 1024;

struct LimitedRows(Vec<u8>);
impl Write for LimitedRows {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_INTERMEDIATE_BYTES {
            return Err(io::Error::other("derived JOIN result exceeds 16 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Execute the two grouping stages while the caller holds the database read lock.
pub fn execute_derived_join_count<W: Write>(
    plan: &DerivedJoinCount,
    left: &Path,
    right: &Path,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let mut intermediate = LimitedRows(Vec::new());
    let mut inner = plan.inner.clone();
    if plan.limit == Some(0) {
        inner.limit = Some(0);
    }
    execute_join(&inner, left, right, &mut intermediate)?;
    let text = std::str::from_utf8(&intermediate.0)
        .map_err(|_| ExecutionError::Invalid("invalid derived JOIN output".into()))?;
    let mut lines = text.lines();
    lines
        .next()
        .ok_or_else(|| ExecutionError::Invalid("missing derived JOIN header".into()))?;
    let mut groups = BTreeMap::<u64, u64>::new();
    for line in lines {
        let (_, count) = line
            .rsplit_once('\t')
            .ok_or_else(|| ExecutionError::Invalid("invalid derived JOIN row".into()))?;
        let count = count
            .parse::<u64>()
            .map_err(|_| ExecutionError::Invalid("invalid derived JOIN count".into()))?;
        let entry = groups.entry(count).or_default();
        *entry = entry
            .checked_add(1)
            .ok_or_else(|| ExecutionError::Invalid("derived JOIN group count overflow".into()))?;
    }
    let mut rows = groups.into_iter().collect::<Vec<_>>();
    if !plan.order.is_empty() {
        rows.sort_by(|left, right| {
            for &(count, descending) in &plan.order {
                let comparison = if count {
                    left.1.cmp(&right.1)
                } else {
                    left.0.cmp(&right.0)
                };
                let comparison = if descending {
                    comparison.reverse()
                } else {
                    comparison
                };
                if !comparison.is_eq() {
                    return comparison;
                }
            }
            std::cmp::Ordering::Equal
        });
    }
    writeln!(output, "{}\t{}", plan.group_name, plan.count_name)?;
    for (value, count) in rows
        .into_iter()
        .skip(plan.offset)
        .take(plan.limit.unwrap_or(usize::MAX))
    {
        writeln!(output, "{value}\t{count}")?;
    }
    Ok(())
}
