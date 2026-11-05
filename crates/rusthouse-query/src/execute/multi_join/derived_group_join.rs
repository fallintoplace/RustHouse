//! Materialize a bounded grouped derived source before its outer join.
use super::ExecutionError;
use crate::join::{DerivedGroupedFilter, DerivedGroupedOrderKey};
use crate::{DerivedGroupedJoin, DerivedGroupedSelect, DerivedUnionJoin, execute_multi_join};
use rusthouse_storage::{Column, write_wide_part};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_DERIVED_BYTES: usize = 64 * 1024 * 1024;
const MAX_DERIVED_ROWS: usize = 262_144;
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct BoundedOutput(Vec<u8>);

impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self
            .0
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("grouped derived result size overflow"))?;
        if next > MAX_DERIVED_BYTES {
            return Err(io::Error::other("grouped derived result exceeds 64 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct TemporaryPart {
    root: PathBuf,
    path: PathBuf,
}

impl Drop for TemporaryPart {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn materialize(
    output: &[u8],
    columns: &[crate::DerivedColumn],
) -> Result<TemporaryPart, ExecutionError> {
    let text = std::str::from_utf8(output)
        .map_err(|_| invalid("grouped derived output is not valid UTF-8"))?;
    let mut lines = text.split_terminator('\n');
    let header = lines
        .next()
        .ok_or_else(|| invalid("grouped derived output has no header"))?;
    if header
        .split('\t')
        .ne(columns.iter().map(|column| column.name.as_str()))
    {
        return Err(invalid(
            "grouped derived output columns differ from its plan",
        ));
    }
    let mut rows = Vec::new();
    for line in lines {
        if rows.len() == MAX_DERIVED_ROWS {
            return Err(invalid("grouped derived result exceeds 262144 rows"));
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != columns.len() {
            return Err(invalid("grouped derived output has an invalid row width"));
        }
        let mut row = Vec::with_capacity(fields.len());
        for field in fields {
            if field == r"\N" {
                row.push(None);
            } else {
                let value = rusthouse_format::decode_tsv_field(field.as_bytes())
                    .map_err(|_| invalid("grouped derived output contains invalid TSV"))?;
                row.push(Some(String::from_utf8(value).map_err(|_| {
                    invalid("grouped derived value is not valid UTF-8")
                })?));
            }
        }
        rows.push(row);
    }

    let mut root = None;
    for _ in 0..16 {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let candidate =
            std::env::temp_dir().join(format!("rusthouse-derived-{}-{id}", std::process::id()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => {
                root = Some(candidate);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    let root = root.ok_or_else(|| invalid("cannot allocate a grouped derived source"))?;
    let path = root.join("part");
    let temporary = TemporaryPart {
        root,
        path: path.clone(),
    };
    let schema = columns
        .iter()
        .map(|column| Column {
            name: column.name.clone(),
            data_type: column.data_type.clone(),
        })
        .collect::<Vec<_>>();
    write_wide_part(&path, &schema, &rows)?;
    Ok(temporary)
}

fn invalid(message: impl Into<String>) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}

/// Execute the inner grouped join, materialize it as a bounded temporary part,
/// and then run the outer join against that part.
pub fn execute_derived_grouped_join<W: Write>(
    plan: &DerivedGroupedJoin,
    inner_paths: &[PathBuf],
    outer_paths: &[PathBuf],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let inner = plan.inner();
    if inner_paths.len() != inner.steps.len() + 2 {
        return Err(invalid(
            "derived grouped source count differs from its plan",
        ));
    }
    let mut grouped_output = BoundedOutput(Vec::new());
    execute_multi_join(inner, inner_paths, &mut grouped_output)?;
    let temporary = materialize(&grouped_output.0, plan.columns())?;

    let outer = plan.outer();
    if outer_paths.len() != outer.steps.len() + 2 {
        return Err(invalid("derived outer source count differs from its plan"));
    }
    let table_names = [&outer.left, &outer.right]
        .into_iter()
        .chain(outer.steps.iter().map(|step| &step.table));
    let paths = table_names
        .zip(outer_paths)
        .map(|(table, path)| {
            if table.eq_ignore_ascii_case(plan.table()) {
                temporary.path.clone()
            } else {
                path.clone()
            }
        })
        .collect::<Vec<_>>();
    execute_multi_join(outer, &paths, output)
}

fn field_number(field: &str) -> Option<f64> {
    (field != r"\N")
        .then(|| field.parse::<f64>().ok())
        .flatten()
}

fn ratio_literal_value(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/').unwrap_or((value, "1"));
    let numerator = numerator.parse::<f64>().ok()?;
    let denominator = denominator.parse::<f64>().ok()?;
    if !numerator.is_finite() || !denominator.is_finite() || denominator == 0.0 {
        return None;
    }
    let result = numerator / denominator;
    result.is_finite().then_some(result)
}

fn numeric_type(name: &str) -> bool {
    ["INT", "UINT", "DECIMAL", "NUMERIC", "FLOAT", "DOUBLE"]
        .iter()
        .any(|kind| name.to_ascii_uppercase().contains(kind))
}

fn compare_fields(
    left: &str,
    right: &str,
    data_type: &str,
    descending: bool,
    nulls_first: Option<bool>,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left == r"\N", right == r"\N") {
        (true, true) => return Ordering::Equal,
        (true, false) => {
            return if nulls_first.unwrap_or(false) {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        (false, true) => {
            return if nulls_first.unwrap_or(false) {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
        (false, false) => {}
    }
    let order = if numeric_type(data_type) {
        left.parse::<f64>()
            .ok()
            .zip(right.parse::<f64>().ok())
            .and_then(|(left, right)| left.partial_cmp(&right))
            .unwrap_or_else(|| left.cmp(right))
    } else {
        let left = rusthouse_format::decode_tsv_field(left.as_bytes())
            .unwrap_or_else(|_| left.as_bytes().to_vec());
        let right = rusthouse_format::decode_tsv_field(right.as_bytes())
            .unwrap_or_else(|_| right.as_bytes().to_vec());
        left.cmp(&right)
    };
    if descending { order.reverse() } else { order }
}

/// Execute grouped window-average deviation filters used by TPC-DS Q53/Q63/Q89.
pub fn execute_derived_grouped_select<W: Write>(
    plan: &DerivedGroupedSelect,
    inner_paths: &[PathBuf],
    output: &mut W,
) -> Result<(), ExecutionError> {
    if inner_paths.len() != plan.inner.steps.len() + 2 {
        return Err(invalid(
            "derived grouped source count differs from its plan",
        ));
    }
    let mut grouped_output = BoundedOutput(Vec::new());
    execute_multi_join(&plan.inner, inner_paths, &mut grouped_output)?;
    let text = std::str::from_utf8(&grouped_output.0)
        .map_err(|_| invalid("derived grouped result is not valid UTF-8"))?;
    let mut lines = text.split_terminator('\n');
    let header = lines
        .next()
        .ok_or_else(|| invalid("derived grouped result has no header"))?;
    if header
        .split('\t')
        .ne(plan.columns.iter().map(|column| column.name.as_str()))
    {
        return Err(invalid(
            "derived grouped result columns differ from its plan",
        ));
    }
    #[derive(Clone, Copy)]
    enum Filter {
        WindowDeviation {
            average: usize,
            sum: usize,
            threshold: f64,
        },
        RatioBetween {
            numerator: usize,
            denominator: usize,
            scale: f64,
            lower: f64,
            upper: f64,
        },
    }
    let filter = match &plan.filter {
        DerivedGroupedFilter::WindowDeviation {
            average_index,
            sum_index,
            threshold,
        } => Filter::WindowDeviation {
            average: *average_index,
            sum: *sum_index,
            threshold: threshold
                .parse::<f64>()
                .map_err(|_| invalid("invalid derived grouped threshold"))?,
        },
        DerivedGroupedFilter::RatioBetween {
            numerator_index,
            denominator_index,
            scale,
            lower,
            upper,
        } => Filter::RatioBetween {
            numerator: *numerator_index,
            denominator: *denominator_index,
            scale: ratio_literal_value(scale)
                .ok_or_else(|| invalid("invalid derived grouped ratio scale"))?,
            lower: ratio_literal_value(lower)
                .ok_or_else(|| invalid("invalid derived grouped ratio bound"))?,
            upper: ratio_literal_value(upper)
                .ok_or_else(|| invalid("invalid derived grouped ratio bound"))?,
        },
    };
    let mut rows = Vec::<Vec<String>>::new();
    for line in lines {
        if rows.len() == MAX_DERIVED_ROWS {
            return Err(invalid("derived grouped result exceeds 262144 rows"));
        }
        let fields = line.split('\t').map(str::to_owned).collect::<Vec<_>>();
        if fields.len() != plan.columns.len() {
            return Err(invalid("derived grouped result has an invalid row width"));
        }
        let keep = match filter {
            Filter::WindowDeviation {
                average,
                sum,
                threshold,
            } => field_number(&fields[average])
                .zip(field_number(&fields[sum]))
                .is_some_and(|(average, sum)| {
                    average > 0.0 && ((sum - average).abs() / average) > threshold
                }),
            Filter::RatioBetween {
                numerator,
                denominator,
                scale,
                lower,
                upper,
            } => field_number(&fields[numerator])
                .zip(field_number(&fields[denominator]))
                .is_some_and(|(numerator, denominator)| {
                    denominator > 0.0
                        && (numerator * scale / denominator) >= lower
                        && (numerator * scale / denominator) <= upper
                }),
        };
        if keep {
            rows.push(fields);
        }
    }

    rows.sort_by(|left, right| {
        for order in &plan.order {
            let comparison = match order.key {
                DerivedGroupedOrderKey::Column(index) => compare_fields(
                    &left[index],
                    &right[index],
                    &plan.columns[index].data_type,
                    order.descending,
                    order.nulls_first,
                ),
                DerivedGroupedOrderKey::Difference {
                    left: first,
                    right: second,
                } => {
                    let left_value = field_number(&left[first])
                        .zip(field_number(&left[second]))
                        .map(|(left, right)| left - right);
                    let right_value = field_number(&right[first])
                        .zip(field_number(&right[second]))
                        .map(|(left, right)| left - right);
                    let comparison = match (left_value, right_value) {
                        (None, None) => std::cmp::Ordering::Equal,
                        (None, Some(_)) => {
                            if order.nulls_first.unwrap_or(false) {
                                std::cmp::Ordering::Less
                            } else {
                                std::cmp::Ordering::Greater
                            }
                        }
                        (Some(_), None) => {
                            if order.nulls_first.unwrap_or(false) {
                                std::cmp::Ordering::Greater
                            } else {
                                std::cmp::Ordering::Less
                            }
                        }
                        (Some(left), Some(right)) => left
                            .partial_cmp(&right)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    };
                    if left_value.is_some() && right_value.is_some() && order.descending {
                        comparison.reverse()
                    } else {
                        comparison
                    }
                }
            };
            if !comparison.is_eq() {
                return comparison;
            }
        }
        std::cmp::Ordering::Equal
    });

    writeln!(output, "{header}")?;
    let start = plan.offset.min(rows.len());
    let end = plan.limit.map_or(rows.len(), |limit| {
        start.saturating_add(limit).min(rows.len())
    });
    for row in &rows[start..end] {
        writeln!(output, "{}", row.join("\t"))?;
    }
    Ok(())
}

/// Execute each UNION ALL branch into a bounded temporary source, then join it.
pub fn execute_derived_union_join<W: Write>(
    plan: &DerivedUnionJoin,
    branch_paths: &[Vec<PathBuf>],
    outer_paths: &[PathBuf],
    output: &mut W,
) -> Result<(), ExecutionError> {
    let branches = plan.branches();
    if branch_paths.len() != branches.len() {
        return Err(invalid("derived UNION branch count differs from its plan"));
    }
    let mut combined = BoundedOutput(Vec::new());
    let expected_header = format!(
        "{}\n",
        plan.columns()
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>()
            .join("\t")
    );
    for (branch_index, (branch, paths)) in branches.iter().zip(branch_paths).enumerate() {
        if paths.len() != branch.steps.len() + 2 {
            return Err(invalid("derived UNION source count differs from its plan"));
        }
        let mut branch_output = BoundedOutput(Vec::new());
        execute_multi_join(branch, paths, &mut branch_output)?;
        let header_end = branch_output
            .0
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(|| invalid("derived UNION branch has no header"))?;
        let header = std::str::from_utf8(&branch_output.0[..header_end])
            .map_err(|_| invalid("derived UNION branch header is not valid UTF-8"))?;
        if (branch_index == 0 && branch_output.0[..=header_end] != expected_header.as_bytes()[..])
            || (branch_index != 0 && header.split('\t').count() != plan.columns().len())
        {
            return Err(invalid("derived UNION branch columns differ from its plan"));
        }
        if combined.0.is_empty() {
            combined.write_all(&branch_output.0[..=header_end])?;
        }
        combined.write_all(&branch_output.0[header_end + 1..])?;
    }
    let temporary = materialize(&combined.0, plan.columns())?;

    let outer = plan.outer();
    if outer_paths.len() != outer.steps.len() + 2 {
        return Err(invalid("derived outer source count differs from its plan"));
    }
    let table_names = [&outer.left, &outer.right]
        .into_iter()
        .chain(outer.steps.iter().map(|step| &step.table));
    let paths = table_names
        .zip(outer_paths)
        .map(|(table, path)| {
            if table.eq_ignore_ascii_case(plan.table()) {
                temporary.path.clone()
            } else {
                path.clone()
            }
        })
        .collect::<Vec<_>>();
    execute_multi_join(outer, &paths, output)
}
