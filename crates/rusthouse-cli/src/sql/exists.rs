use rusthouse_query::{CorrelatedExists, Operator, Predicate, Projection, Select, Source, execute};
use std::io::{self, Write};
use std::path::Path;

const MAX_KEYS_BYTES: usize = 16 * 1024 * 1024;
struct KeyOutput(Vec<u8>);
impl Write for KeyOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_KEYS_BYTES {
            return Err(io::Error::other("correlated EXISTS keys exceed 16 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn valid_integer_key(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| byte.is_ascii_digit() || (index == 0 && byte == b'-'))
}

fn schema_column<'a>(
    name: &str,
    schema: &'a [rusthouse_query::ColumnDef],
) -> Option<&'a rusthouse_query::ColumnDef> {
    schema.iter().find(|c| c.name == name)
}

fn base_type(s: &str) -> &str {
    s.strip_prefix("Nullable(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(s)
}

/// Resolve an equality correlation using both table schemas and materialize
/// distinct inner keys. Callers hold the database read lock for both scans.
pub(super) fn bind(database: &Path, query: CorrelatedExists) -> Result<Select, String> {
    let CorrelatedExists {
        mut outer,
        mut inner,
        negated,
    } = query;
    let (Some(Source::Table(outer_table)), Some(Source::Table(inner_table))) =
        (&outer.source, &inner.source)
    else {
        return Err("correlated EXISTS requires two stored tables".into());
    };
    let outer_schema = crate::catalog::read_schema(database, outer_table)?;
    let inner_schema = crate::catalog::read_schema(database, inner_table)?;
    let mut correlation = None;
    for (index, predicate) in inner.predicates.iter().enumerate() {
        if let Predicate::ColumnComparison {
            column: left,
            operator: Operator::Eq,
            other: right,
        } = predicate
        {
            let left_inner = schema_column(left, &inner_schema).is_some();
            let right_inner = schema_column(right, &inner_schema).is_some();
            let left_outer = schema_column(left, &outer_schema).is_some();
            let right_outer = schema_column(right, &outer_schema).is_some();
            let pair = if left_inner && !left_outer && right_outer && !right_inner {
                Some((left.clone(), right.clone()))
            } else if right_inner && !right_outer && left_outer && !left_inner {
                Some((right.clone(), left.clone()))
            } else {
                None
            };
            if let Some((inner_key, outer_key)) = pair {
                if correlation.replace((index, inner_key, outer_key)).is_some() {
                    return Err("correlated EXISTS supports one equality key".into());
                }
            }
        }
    }
    let (index, inner_key, outer_key) = correlation
        .ok_or("correlated EXISTS requires an unambiguous equality between both tables")?;
    let inner_type = &schema_column(&inner_key, &inner_schema)
        .expect("key exists")
        .data_type;
    let outer_type = &schema_column(&outer_key, &outer_schema)
        .expect("key exists")
        .data_type;
    let inner_base = base_type(inner_type);
    if inner_base != base_type(outer_type)
        || !matches!(
            inner_base,
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
    {
        return Err("correlated EXISTS requires integer keys with matching types".into());
    }
    inner.predicates.remove(index);
    inner.projection = Projection::Columns(vec![inner_key]);
    inner.distinct = true;
    let path = crate::database::select_source(database, inner_table)?;
    inner.source = Some(Source::Table("part".into()));
    let mut output = KeyOutput(Vec::new());
    execute(&inner, Some(&path), &mut output).map_err(|error| error.to_string())?;
    let result = std::str::from_utf8(&output.0).map_err(|error| error.to_string())?;
    let mut values = Vec::new();
    for value in result.lines().skip(1) {
        if value == "\\N" || value == "NULL" {
            continue;
        }
        if !valid_integer_key(value) {
            return Err("invalid integer from correlated EXISTS".into());
        }
        values.push(value.to_owned());
    }
    if negated {
        // NULL never equals an inner key, so it always satisfies NOT EXISTS.
        let mut null_branch = outer.predicates.clone();
        null_branch.push(Predicate::IsNull {
            column: outer_key.clone(),
            negated: false,
        });
        outer.or_predicates.push(null_branch);
        outer.predicates.push(Predicate::NotIn {
            column: outer_key,
            values,
        });
    } else {
        outer.predicates.push(Predicate::In {
            column: outer_key,
            values,
        });
    }
    Ok(outer)
}
