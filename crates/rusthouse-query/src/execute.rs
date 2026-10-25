use crate::{Operator, Projection, Select};
use rusthouse_storage::{NumericColumn, NumericPartReader, PartError, inspect_part};
use std::error::Error;
use std::fmt;
use std::io::{self, BufWriter, Write};
use std::path::Path;

#[derive(Debug)]
pub enum ExecutionError {
    Storage(PartError),
    Io(io::Error),
    Invalid(String),
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Invalid(message) => write!(f, "SQL execution error: {message}"),
        }
    }
}

impl Error for ExecutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Invalid(_) => None,
        }
    }
}

impl From<PartError> for ExecutionError {
    fn from(error: PartError) -> Self {
        Self::Storage(error)
    }
}

impl From<io::Error> for ExecutionError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Copy)]
enum Number {
    Unsigned(u128),
    Signed(i128),
    Float(f64),
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsigned(value) => write!(f, "{value}"),
            Self::Signed(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "{value}"),
        }
    }
}

pub fn execute<W: Write>(
    query: &Select,
    part: Option<&Path>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    let mut buffered = BufWriter::new(output);
    execute_into(query, part, &mut buffered)?;
    buffered.flush()?;
    Ok(())
}

fn execute_into<W: Write>(
    query: &Select,
    part: Option<&Path>,
    output: &mut W,
) -> Result<(), ExecutionError> {
    if let Projection::Literal(value) = query.projection {
        writeln!(output, "{value}")?;
        if query.limit != Some(0) {
            writeln!(output, "{value}")?;
        }
        return Ok(());
    }

    if query.source.as_deref() != Some("part") {
        return Err(ExecutionError::Invalid(
            "only FROM part is supported".into(),
        ));
    }
    let part = part.ok_or_else(|| {
        ExecutionError::Invalid("pass --part <directory> for table queries".into())
    })?;
    let metadata = inspect_part(part)?;

    if matches!(query.projection, Projection::Count) && query.predicate.is_none() {
        writeln!(output, "count")?;
        if query.limit != Some(0) {
            writeln!(output, "{}", metadata.rows)?;
        }
        return Ok(());
    }

    let projected = match &query.projection {
        Projection::All => metadata
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>(),
        Projection::Columns(names) => names.clone(),
        Projection::Count => Vec::new(),
        Projection::Literal(_) => unreachable!(),
    };
    let mut scan_columns = Vec::new();
    let projected_indices = projected
        .iter()
        .map(|name| {
            if let Some(index) = scan_columns.iter().position(|column| column == name) {
                index
            } else {
                let index = scan_columns.len();
                scan_columns.push(name.clone());
                index
            }
        })
        .collect::<Vec<_>>();
    let predicate_index = query.predicate.as_ref().map(|predicate| {
        if let Some(index) = scan_columns
            .iter()
            .position(|name| *name == predicate.column)
        {
            index
        } else {
            scan_columns.push(predicate.column.clone());
            scan_columns.len() - 1
        }
    });
    let comparison = query
        .predicate
        .as_ref()
        .map(|predicate| {
            let column = metadata
                .columns
                .iter()
                .find(|column| column.name == predicate.column)
                .ok_or_else(|| {
                    ExecutionError::Invalid(format!("unknown WHERE column: {}", predicate.column))
                })?;
            parse_numeric(&column.data_type, &predicate.value)
        })
        .transpose()?;
    let mut reader = NumericPartReader::open(part, &scan_columns)?;

    let is_count = matches!(query.projection, Projection::Count);
    if is_count {
        writeln!(output, "count")?;
    } else {
        writeln!(output, "{}", projected.join("\t"))?;
    }
    if query.limit == Some(0) {
        return Ok(());
    }

    let limit = query.limit.unwrap_or(usize::MAX);
    let mut emitted = 0usize;
    let mut matched = 0u64;
    while let Some(batch) = reader.next_batch()? {
        let rows = batch[0].len();
        for row in 0..rows {
            if let (Some(predicate), Some(index), Some(target)) =
                (&query.predicate, predicate_index, comparison)
            {
                if !number_matches(number_at(&batch[index], row), target, predicate.operator) {
                    continue;
                }
            }
            if is_count {
                matched += 1;
            } else {
                for (column_number, index) in projected_indices.iter().enumerate() {
                    if column_number != 0 {
                        write!(output, "\t")?;
                    }
                    write!(output, "{}", number_at(&batch[*index], row))?;
                }
                writeln!(output)?;
                emitted += 1;
                if emitted == limit {
                    return Ok(());
                }
            }
        }
    }

    if is_count {
        writeln!(output, "{matched}")?;
    }
    Ok(())
}

fn parse_numeric(data_type: &str, text: &str) -> Result<Number, ExecutionError> {
    let value = match data_type {
        "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128" => {
            text.parse::<u128>().ok().map(Number::Unsigned)
        }
        "Int8" | "Int16" | "Int32" | "Int64" | "Int128" => {
            text.parse::<i128>().ok().map(Number::Signed)
        }
        "Float32" => text
            .parse::<f32>()
            .ok()
            .map(|value| Number::Float(f64::from(value))),
        "Float64" => text.parse::<f64>().ok().map(Number::Float),
        other => {
            return Err(ExecutionError::Invalid(format!(
                "unsupported WHERE column type: {other}"
            )));
        }
    };
    value.ok_or_else(|| ExecutionError::Invalid(format!("invalid numeric WHERE value: {text}")))
}

fn number_at(column: &NumericColumn, row: usize) -> Number {
    match column {
        NumericColumn::UInt8(values) => Number::Unsigned(u128::from(values[row])),
        NumericColumn::UInt16(values) => Number::Unsigned(u128::from(values[row])),
        NumericColumn::UInt32(values) => Number::Unsigned(u128::from(values[row])),
        NumericColumn::UInt64(values) => Number::Unsigned(u128::from(values[row])),
        NumericColumn::UInt128(values) => Number::Unsigned(values[row]),
        NumericColumn::Int8(values) => Number::Signed(i128::from(values[row])),
        NumericColumn::Int16(values) => Number::Signed(i128::from(values[row])),
        NumericColumn::Int32(values) => Number::Signed(i128::from(values[row])),
        NumericColumn::Int64(values) => Number::Signed(i128::from(values[row])),
        NumericColumn::Int128(values) => Number::Signed(values[row]),
        NumericColumn::Float32(values) => Number::Float(f64::from(values[row])),
        NumericColumn::Float64(values) => Number::Float(values[row]),
    }
}

fn number_matches(value: Number, target: Number, operator: Operator) -> bool {
    match (value, target) {
        (Number::Unsigned(value), Number::Unsigned(target)) => compare(value, target, operator),
        (Number::Signed(value), Number::Signed(target)) => compare(value, target, operator),
        (Number::Float(value), Number::Float(target)) => compare(value, target, operator),
        _ => false,
    }
}

fn compare<T: PartialOrd>(value: T, target: T, operator: Operator) -> bool {
    match operator {
        Operator::Eq => value == target,
        Operator::Ne => value != target,
        Operator::Lt => value < target,
        Operator::Le => value <= target,
        Operator::Gt => value > target,
        Operator::Ge => value >= target,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_select;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    fn run(query: &str, part: Option<&Path>) -> Result<String, ExecutionError> {
        let select = parse_select(query).unwrap();
        let mut output = Vec::new();
        execute(&select, part, &mut output)?;
        Ok(String::from_utf8(output).unwrap())
    }

    #[test]
    fn runs_scalar_queries_without_a_part() {
        assert_eq!(run("SELECT 1", None).unwrap(), "1\n1\n");
        assert_eq!(run("SELECT -5 LIMIT 0", None).unwrap(), "-5\n");
    }

    #[test]
    fn count_without_where_uses_part_metadata() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-count-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("count.txt"), "42\n").unwrap();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n1 columns:\n\u{0060}city\u{0060} String\n",
        )
        .unwrap();
        assert_eq!(
            run("SELECT count(*) FROM part", Some(&dir)).unwrap(),
            "count\n42\n"
        );
        assert_eq!(
            run("SELECT count(*) FROM part LIMIT 0", Some(&dir)).unwrap(),
            "count\n"
        );
        assert!(run("SELECT city FROM part", Some(&dir)).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn checks_predicates_on_empty_parts_and_before_limit_zero() {
        let dir = std::env::temp_dir().join(format!(
            "rusthouse-empty-where-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("count.txt"), "0\n").unwrap();
        fs::write(
            dir.join("columns.txt"),
            "columns format version: 1\n1 columns:\n`id` UInt64\n",
        )
        .unwrap();

        for statement in [
            "SELECT id FROM part WHERE id = -1",
            "SELECT id FROM part WHERE id = -1 LIMIT 0",
            "SELECT count(*) FROM part WHERE id = -1",
        ] {
            let select = parse_select(statement).unwrap();
            let mut output = Vec::new();
            assert!(
                matches!(
                    execute(&select, Some(&dir), &mut output),
                    Err(ExecutionError::Invalid(_))
                ),
                "{statement}"
            );
            assert!(output.is_empty());
        }

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_unknown_sources_and_missing_part_directory() {
        assert!(matches!(
            run("SELECT id FROM other", Some(Path::new("."))),
            Err(ExecutionError::Invalid(_))
        ));
        assert!(matches!(
            run("SELECT id FROM part", None),
            Err(ExecutionError::Invalid(_))
        ));
    }

    #[test]
    fn numeric_comparisons_do_not_lose_large_integer_precision() {
        assert!(number_matches(
            Number::Unsigned(u128::MAX),
            Number::Unsigned(u128::MAX - 1),
            Operator::Gt,
        ));
        assert!(number_matches(
            Number::Signed(i128::MIN),
            Number::Signed(0),
            Operator::Lt,
        ));
        assert!(!number_matches(
            Number::Float(f64::NAN),
            Number::Float(1.0),
            Operator::Eq,
        ));
        assert!(number_matches(
            number_at(&NumericColumn::UInt128(vec![u128::MAX]), 0),
            parse_numeric("UInt128", &u128::MAX.to_string()).unwrap(),
            Operator::Eq,
        ));
        assert!(number_matches(
            number_at(&NumericColumn::Int128(vec![i128::MIN]), 0),
            parse_numeric("Int128", &i128::MIN.to_string()).unwrap(),
            Operator::Eq,
        ));
    }
}
