//! Run a bounded cross join of scalar aggregate subqueries in `FROM`.
//!
//! Each input must produce exactly one row. The intermediate wide part lets
//! the normal SELECT output path handle headers, escaping, and output formats.

use rusthouse_query::{
    Projection, Source, execute, execute_join, execute_multi_join, has_explicit_join,
    has_implicit_join, parse_join_with_schema, parse_select,
};
use rusthouse_storage::{Column, write_wide_part};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_DERIVED_SOURCES: usize = 16;
const MAX_DERIVED_COLUMNS: usize = 128;
const MAX_DERIVED_BYTES: usize = 8 * 1024 * 1024;
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(super) struct ScalarDerivedCross {
    queries: Vec<String>,
    outputs: Option<Vec<ScalarOutput>>,
    pub(super) limit: Option<usize>,
    pub(super) offset: Option<usize>,
}

#[derive(Debug, Clone)]
struct ScalarOutput {
    name: String,
    expression: OutputExpression,
}

#[derive(Debug, Clone)]
enum OutputExpression {
    Scalar(rusthouse_query::ScalarExpr),
    NullWhen {
        condition: rusthouse_query::ScalarExpr,
        otherwise: rusthouse_query::ScalarExpr,
    },
}

pub(super) struct TemporaryPart {
    root: PathBuf,
    path: PathBuf,
    pub(super) output_rename: Option<(String, String)>,
}

impl TemporaryPart {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryPart {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$')
}

fn keyword_at(sql: &str, index: usize, keyword: &str) -> bool {
    let bytes = sql.as_bytes();
    let end = index.saturating_add(keyword.len());
    if end > bytes.len()
        || !bytes
            .get(index..end)
            .is_some_and(|word| word.eq_ignore_ascii_case(keyword.as_bytes()))
    {
        return false;
    }
    (index == 0 || !is_identifier_byte(bytes[index - 1]))
        && (end == bytes.len() || !is_identifier_byte(bytes[end]))
}

fn skip_quoted(sql: &str, index: usize, quote: u8) -> usize {
    let bytes = sql.as_bytes();
    let mut cursor = index + 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = (cursor + 2).min(bytes.len());
        } else if bytes[cursor] == quote {
            if bytes.get(cursor + 1) == Some(&quote) {
                cursor += 2;
            } else {
                return cursor + 1;
            }
        } else {
            cursor += 1;
        }
    }
    bytes.len()
}

fn find_top_level_keyword(sql: &str, keyword: &str, start: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut cursor = start;
    let mut depth = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            b'(' => {
                depth += 1;
                cursor += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                cursor += 1;
            }
            _ if depth == 0 && keyword_at(sql, cursor, keyword) => return Some(cursor),
            _ => cursor += 1,
        }
    }
    None
}

fn matching_close(sql: &str, open: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut cursor = open;
    let mut depth = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            b'(' => {
                depth += 1;
                cursor += 1;
            }
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(cursor);
                }
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    None
}

fn skip_space(sql: &str, mut cursor: usize) -> usize {
    while sql
        .as_bytes()
        .get(cursor)
        .is_some_and(u8::is_ascii_whitespace)
    {
        cursor += 1;
    }
    cursor
}

fn parse_alias(sql: &str, mut cursor: usize) -> Option<usize> {
    cursor = skip_space(sql, cursor);
    if keyword_at(sql, cursor, "AS") {
        cursor = skip_space(sql, cursor + 2);
    }
    let bytes = sql.as_bytes();
    match *bytes.get(cursor)? {
        b'"' | b'`' => Some(skip_quoted(sql, cursor, bytes[cursor])),
        byte if byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$') => {
            cursor += 1;
            while bytes
                .get(cursor)
                .is_some_and(|byte| is_identifier_byte(*byte))
            {
                cursor += 1;
            }
            Some(cursor)
        }
        _ => None,
    }
}

fn parse_tail(sql: &str) -> Option<(Option<usize>, Option<usize>)> {
    let tail = sql.trim();
    if tail.is_empty() {
        return Some((None, None));
    }
    let mut rest = tail;
    if keyword_at(rest, 0, "ORDER") {
        let by = rest.get(5..)?.trim_start();
        if !keyword_at(by, 0, "BY") {
            return None;
        }
        let order_end = find_top_level_keyword(by, "LIMIT", 2);
        if let Some(index) = order_end {
            rest = by[index..].trim_start();
        } else {
            return Some((None, None));
        }
    }
    if keyword_at(rest, 0, "LIMIT") {
        let mut words = rest[5..].split_whitespace();
        let limit = words.next()?.parse::<usize>().ok()?;
        let mut offset = None;
        if let Some(word) = words.next() {
            if !word.eq_ignore_ascii_case("OFFSET") {
                return None;
            }
            offset = Some(words.next()?.parse::<usize>().ok()?);
        }
        if words.next().is_some() {
            return None;
        }
        return Some((Some(limit), offset));
    }
    None
}

fn has_aggregate_projection(query: &str) -> bool {
    let Some(from) = find_top_level_keyword(query, "FROM", 6) else {
        return false;
    };
    ["COUNT", "SUM", "AVG", "MIN", "MAX"]
        .iter()
        .any(|name| find_top_level_keyword(&query[6..from], name, 0).is_some())
}

fn skip_leading_comments(mut sql: &str) -> &str {
    loop {
        sql = sql.trim_start();
        if sql.starts_with("--") || sql.starts_with('#') {
            let end = sql.find(['\r', '\n']).unwrap_or(sql.len());
            sql = &sql[end..];
        } else if sql.starts_with("/*") {
            let Some(end) = sql.find("*/") else {
                return sql;
            };
            sql = &sql[end + 2..];
        } else {
            return sql;
        }
    }
}

fn split_top_level(sql: &str, delimiter: u8) -> Vec<&str> {
    let bytes = sql.as_bytes();
    let mut items = Vec::new();
    let mut start = 0;
    let mut cursor = 0;
    let mut depth = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            b'(' => {
                depth += 1;
                cursor += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                cursor += 1;
            }
            byte if depth == 0 && byte == delimiter => {
                items.push(sql[start..cursor].trim());
                start = cursor + 1;
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    items.push(sql[start..].trim());
    items
}

fn valid_alias(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && (bytes[0].is_ascii_alphabetic() || matches!(bytes[0], b'_' | b'$'))
        && bytes[1..].iter().all(|byte| is_identifier_byte(*byte))
}

fn strip_output_alias(item: &str) -> (&str, Option<String>) {
    if let Some(index) = find_top_level_keyword(item, "AS", 0) {
        let alias = item[index + 2..].trim();
        if valid_alias(alias) {
            return (item[..index].trim(), Some(alias.to_owned()));
        }
        return (item, None);
    }
    let bytes = item.as_bytes();
    let mut cursor = 0;
    let mut depth = 0usize;
    let mut last_space = None;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(item, cursor, bytes[cursor]),
            b'(' => {
                depth += 1;
                cursor += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                cursor += 1;
            }
            byte if depth == 0 && byte.is_ascii_whitespace() => {
                last_space = Some(cursor);
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    if let Some(index) = last_space {
        let alias = item[index..].trim();
        if valid_alias(alias) {
            return (item[..index].trim(), Some(alias.to_owned()));
        }
    }
    (item, None)
}

fn find_cast_start(sql: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            _ if keyword_at(sql, cursor, "CAST") => {
                let open = skip_space(sql, cursor + 4);
                if bytes.get(open) == Some(&b'(') {
                    return Some(cursor);
                }
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    None
}

fn normalize_type_name(type_name: &str) -> String {
    let type_name = type_name.trim();
    if let Some(open) = type_name.find('(') {
        if type_name.ends_with(')') {
            let base = type_name[..open].trim().to_ascii_uppercase();
            let parameters = type_name[open + 1..type_name.len() - 1]
                .split(',')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(", ");
            return format!("{base}({parameters})");
        }
    }
    type_name.to_ascii_uppercase()
}

fn strip_casts(sql: &str) -> Result<(String, HashMap<String, String>), String> {
    let mut expression = sql.to_owned();
    let mut casts = HashMap::new();
    for _ in 0..16 {
        let Some(start) = find_cast_start(&expression) else {
            return Ok((expression, casts));
        };
        let open = skip_space(&expression, start + 4);
        let Some(close) = matching_close(&expression, open) else {
            return Err("CAST expression is missing ')'".into());
        };
        let body = &expression[open + 1..close];
        let Some(as_index) = find_top_level_keyword(body, "AS", 0) else {
            return Err("CAST expression requires AS".into());
        };
        let value = body[..as_index].trim().to_owned();
        let type_name = normalize_type_name(&body[as_index + 2..]);
        if value.is_empty() || type_name.is_empty() {
            return Err("CAST expression requires a value and type".into());
        }
        if valid_alias(&value) {
            casts.insert(
                value.to_ascii_lowercase(),
                format!("CAST({value} AS {type_name})"),
            );
        }
        expression.replace_range(start..close + 1, &value);
    }
    if find_cast_start(&expression).is_some() {
        return Err("scalar expression exceeds 16 CAST operations".into());
    }
    Ok((expression, casts))
}

fn parse_output_expression(
    expression: &str,
    casts: &HashMap<String, String>,
) -> Result<OutputExpression, String> {
    let expression = expression.trim();
    if expression
        .get(..4)
        .is_some_and(|head| head.eq_ignore_ascii_case("CASE"))
    {
        let body = expression[4..].trim_start();
        let Some(when) = find_top_level_keyword(body, "WHEN", 0) else {
            return Err("outer CASE expression requires WHEN".into());
        };
        let Some(then) = find_top_level_keyword(body, "THEN", when + 4) else {
            return Err("outer CASE expression requires THEN".into());
        };
        let Some(otherwise) = find_top_level_keyword(body, "ELSE", then + 4) else {
            return Err("outer CASE expression requires ELSE".into());
        };
        if !body[then + 4..otherwise]
            .trim()
            .eq_ignore_ascii_case("NULL")
        {
            return rusthouse_query::parse_scalar_expression(expression)
                .map(OutputExpression::Scalar)
                .map_err(|error| error.to_string());
        }
        let Some(end) = find_top_level_keyword(body, "END", otherwise + 4) else {
            return Err("outer CASE expression requires END".into());
        };
        if !body[end + 3..].trim().is_empty() {
            return Err("unexpected text after outer CASE expression".into());
        }
        let condition = rusthouse_query::parse_scalar_expression(body[when + 4..then].trim())
            .map_err(|error| error.to_string())?;
        let otherwise = rusthouse_query::parse_scalar_expression(body[otherwise + 4..end].trim())
            .map_err(|error| error.to_string())?;
        return Ok(OutputExpression::NullWhen {
            condition,
            otherwise,
        });
    }
    let _ = casts;
    rusthouse_query::parse_scalar_expression(expression)
        .map(OutputExpression::Scalar)
        .map_err(|error| error.to_string())
}

fn render_default_name(
    expression: &rusthouse_query::ScalarExpr,
    casts: &HashMap<String, String>,
) -> Option<String> {
    use rusthouse_query::ScalarExpr as Expr;
    match expression {
        Expr::Column(name) => Some(
            casts
                .get(&name.to_ascii_lowercase())
                .cloned()
                .unwrap_or_else(|| name.clone()),
        ),
        Expr::Integer(value) => Some(value.to_string()),
        Expr::Decimal(value) => Some(value.clone()),
        Expr::Add(left, right) => Some(format!(
            "({} + {})",
            render_default_name(left, casts)?,
            render_default_name(right, casts)?
        )),
        Expr::Subtract(left, right) => Some(format!(
            "({} - {})",
            render_default_name(left, casts)?,
            render_default_name(right, casts)?
        )),
        Expr::Multiply(left, right) => Some(format!(
            "({} * {})",
            render_default_name(left, casts)?,
            render_default_name(right, casts)?
        )),
        Expr::Divide(left, right) => Some(format!(
            "({} / {})",
            render_default_name(left, casts)?,
            render_default_name(right, casts)?
        )),
        Expr::Modulo(left, right) => Some(format!(
            "({} % {})",
            render_default_name(left, casts)?,
            render_default_name(right, casts)?
        )),
        _ => None,
    }
}

fn parse_outer_outputs(projection: &str) -> Result<Option<Vec<ScalarOutput>>, String> {
    let mut outputs = Vec::new();
    for item in split_top_level(projection, b',') {
        let (expression, alias) = strip_output_alias(item);
        let (expression, casts) = match strip_casts(expression) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
        let parsed = match parse_output_expression(&expression, &casts) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
        let name = if let Some(alias) = alias {
            alias
        } else {
            match &parsed {
                OutputExpression::Scalar(rusthouse_query::ScalarExpr::Column(name)) => name.clone(),
                OutputExpression::Scalar(expression) => {
                    let Some(name) = render_default_name(expression, &casts) else {
                        return Ok(None);
                    };
                    name
                }
                OutputExpression::NullWhen { .. } => return Ok(None),
            }
        };
        outputs.push(ScalarOutput {
            name,
            expression: parsed,
        });
    }
    if outputs.is_empty() {
        return Ok(None);
    }
    Ok(Some(outputs))
}

/// Recognize a top-level `SELECT * FROM (SELECT aggregate...), ...` shape.
pub(super) fn parse(sql: &str) -> Result<Option<ScalarDerivedCross>, String> {
    let sql = skip_leading_comments(sql)
        .trim()
        .trim_end_matches(';')
        .trim_end();
    let Some(select_end) = sql
        .get(..6)
        .filter(|head| head.eq_ignore_ascii_case("SELECT"))
    else {
        return Ok(None);
    };
    let from = find_top_level_keyword(sql, "FROM", select_end.len());
    let Some(from) = from else {
        return Ok(None);
    };
    let projection = sql[select_end.len()..from].trim();
    let outputs = if projection == "*" {
        None
    } else {
        let Some(outputs) = parse_outer_outputs(projection)? else {
            return Ok(None);
        };
        Some(outputs)
    };
    let mut cursor = skip_space(sql, from + 4);
    let mut queries = Vec::new();
    loop {
        if queries.len() == MAX_DERIVED_SOURCES || sql.as_bytes().get(cursor) != Some(&b'(') {
            return Ok(None);
        }
        let Some(close) = matching_close(sql, cursor) else {
            return Err("scalar aggregate subquery is missing ')'".into());
        };
        let inner = sql[cursor + 1..close].trim();
        if !inner
            .get(..6)
            .is_some_and(|head| head.eq_ignore_ascii_case("SELECT"))
            || !has_aggregate_projection(inner)
        {
            return Ok(None);
        }
        queries.push(inner.to_owned());
        let Some(after_alias) = parse_alias(sql, close + 1) else {
            return Ok(None);
        };
        cursor = skip_space(sql, after_alias);
        if sql.as_bytes().get(cursor) == Some(&b',') {
            cursor = skip_space(sql, cursor + 1);
            continue;
        }
        let Some((limit, offset)) = parse_tail(&sql[cursor..]) else {
            if sql[cursor..].trim().is_empty() {
                return Ok(Some(ScalarDerivedCross {
                    queries,
                    outputs,
                    limit: None,
                    offset: None,
                }));
            }
            return Ok(None);
        };
        return Ok(Some(ScalarDerivedCross {
            queries,
            outputs,
            limit,
            offset,
        }));
    }
}

fn execute_inner_query(query: &str, database: &Path) -> Result<Vec<u8>, String> {
    if has_implicit_join(query).map_err(|error| error.to_string())?
        || has_explicit_join(query).map_err(|error| error.to_string())?
    {
        let join = parse_join_with_schema(query, |table| {
            crate::catalog::read_schema(database, table)
                .map(|columns| columns.into_iter().map(|column| column.name).collect())
                .map_err(rusthouse_query::ParseError)
        })
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "scalar subquery is not a supported join".to_owned())?;
        if (join.aggregate.is_none()
            && join.scalar_aggregates.is_empty()
            && join.scalar_sum.is_none()
            && join.scalar_formula.is_none())
            || join.grouped_count.is_some()
            || join.grouped_sum.is_some()
            || join.grouped_min.is_some()
            || !join.grouped_avg_metrics.is_empty()
            || !join.grouped_case_counts.is_empty()
        {
            return Err("derived FROM inputs must be scalar aggregates".into());
        }
        let tables = std::iter::once(&join.left)
            .chain(std::iter::once(&join.right))
            .chain(join.steps.iter().map(|step| &step.table));
        let paths = tables
            .map(|table| crate::database::select_source(database, table))
            .collect::<Result<Vec<_>, _>>()?;
        let mut output = Vec::new();
        let result = if paths.len() == 2 {
            execute_join(&join, &paths[0], &paths[1], &mut output)
        } else {
            execute_multi_join(&join, &paths, &mut output)
        };
        result.map_err(|error| error.to_string())?;
        Ok(output)
    } else {
        let mut select = parse_select(query).map_err(|error| error.to_string())?;
        if !matches!(
            select.projection,
            Projection::Count
                | Projection::CountColumn { .. }
                | Projection::CountDistinct { .. }
                | Projection::Aggregate { .. }
                | Projection::MultipleAggregates(_)
        ) {
            return Err("derived FROM inputs must be scalar aggregates".into());
        }
        if select.having.is_some() || select.distinct || select.rename.is_some() {
            return Err("derived FROM inputs must be scalar aggregates".into());
        }
        let Some(Source::Table(table)) = select.source.as_ref() else {
            return Err("derived scalar aggregate requires a stored table".into());
        };
        let path = crate::database::select_source(database, table)?;
        select.source = Some(Source::Table("part".into()));
        let mut output = Vec::new();
        execute(&select, Some(&path), &mut output).map_err(|error| error.to_string())?;
        Ok(output)
    }
}

fn decode_single_row(output: &[u8]) -> Result<(Vec<String>, Vec<Option<String>>), String> {
    if output.len() > MAX_DERIVED_BYTES {
        return Err("scalar aggregate output exceeds 8 MiB".into());
    }
    let text = std::str::from_utf8(output)
        .map_err(|_| "scalar aggregate output is not valid UTF-8".to_owned())?;
    let mut lines = text.split_terminator('\n');
    let header = lines
        .next()
        .ok_or_else(|| "scalar aggregate output has no header".to_owned())?;
    let row = lines
        .next()
        .ok_or_else(|| "scalar aggregate output has no result row".to_owned())?;
    if lines.next().is_some() {
        return Err("derived FROM input returned more than one row".into());
    }
    let names = header.split('\t').map(str::to_owned).collect::<Vec<_>>();
    let values = row
        .split('\t')
        .map(|field| {
            if field == r"\N" {
                Ok(None)
            } else {
                let bytes = crate::tsv_escape::decode_tsv_field(field.as_bytes())
                    .map_err(|error| error.to_string())?;
                String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| "scalar aggregate value is not valid UTF-8".to_owned())
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    if names.is_empty() || names.len() != values.len() {
        return Err("scalar aggregate result has an invalid row width".into());
    }
    Ok((names, values))
}

fn allocate_temp_root() -> Result<PathBuf, String> {
    for _ in 0..16 {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rusthouse-scalar-derived-{}-{id}",
            std::process::id()
        ));
        match std::fs::create_dir(&root) {
            Ok(()) => return Ok(root),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("cannot allocate scalar aggregate result".into())
}

#[derive(Clone, Copy)]
enum EvalValue {
    Null,
    Number(f64),
    Boolean(bool),
}

fn numeric(value: EvalValue) -> Option<f64> {
    match value {
        EvalValue::Number(value) => Some(value),
        EvalValue::Boolean(value) => Some(if value { 1.0 } else { 0.0 }),
        EvalValue::Null => None,
    }
}

fn truthy(value: EvalValue) -> bool {
    match value {
        EvalValue::Boolean(value) => value,
        EvalValue::Number(value) => value != 0.0,
        EvalValue::Null => false,
    }
}

fn compare_numbers(left: f64, operator: rusthouse_query::Operator, right: f64) -> bool {
    use rusthouse_query::Operator;
    match operator {
        Operator::Eq => left == right,
        Operator::Ne => left != right,
        Operator::Lt => left < right,
        Operator::Le => left <= right,
        Operator::Gt => left > right,
        Operator::Ge => left >= right,
        Operator::Modulo(_) | Operator::ModuloNe(_) => false,
    }
}

fn eval_scalar(
    expression: &rusthouse_query::ScalarExpr,
    inputs: &HashMap<String, Option<String>>,
) -> Result<EvalValue, String> {
    use rusthouse_query::ScalarExpr as Expr;
    let value = match expression {
        Expr::Column(name) => match inputs
            .get(&name.to_ascii_lowercase())
            .ok_or_else(|| format!("unknown scalar derived column: {name}"))?
        {
            Some(value) => EvalValue::Number(
                value
                    .parse::<f64>()
                    .map_err(|_| format!("scalar derived column is not numeric: {name}"))?,
            ),
            None => EvalValue::Null,
        },
        Expr::Integer(value) => EvalValue::Number(*value as f64),
        Expr::Decimal(value) => EvalValue::Number(
            value
                .parse::<f64>()
                .map_err(|_| "invalid decimal in scalar derived expression".to_owned())?,
        ),
        Expr::Add(left, right)
        | Expr::Subtract(left, right)
        | Expr::Multiply(left, right)
        | Expr::Divide(left, right)
        | Expr::Modulo(left, right) => {
            let left = numeric(eval_scalar(left, inputs)?);
            let right = numeric(eval_scalar(right, inputs)?);
            match (left, right) {
                (Some(left), Some(right)) => {
                    let result = match expression {
                        Expr::Add(..) => left + right,
                        Expr::Subtract(..) => left - right,
                        Expr::Multiply(..) => left * right,
                        Expr::Divide(..) => left / right,
                        Expr::Modulo(..) => left % right,
                        _ => unreachable!(),
                    };
                    EvalValue::Number(result)
                }
                _ => EvalValue::Null,
            }
        }
        Expr::Compare(left, operator, right) => {
            match (
                numeric(eval_scalar(left, inputs)?),
                numeric(eval_scalar(right, inputs)?),
            ) {
                (Some(left), Some(right)) => {
                    EvalValue::Boolean(compare_numbers(left, *operator, right))
                }
                _ => EvalValue::Null,
            }
        }
        Expr::IsNull { column, negated } => {
            let value = inputs
                .get(&column.to_ascii_lowercase())
                .ok_or_else(|| format!("unknown scalar derived column: {column}"))?;
            EvalValue::Boolean(value.is_none() != *negated)
        }
        Expr::And(left, right) => EvalValue::Boolean(
            truthy(eval_scalar(left, inputs)?) && truthy(eval_scalar(right, inputs)?),
        ),
        Expr::Or(left, right) => EvalValue::Boolean(
            truthy(eval_scalar(left, inputs)?) || truthy(eval_scalar(right, inputs)?),
        ),
        Expr::Case {
            branches,
            else_expr,
        } => {
            let mut result = EvalValue::Null;
            let mut matched = false;
            for (condition, value) in branches {
                if truthy(eval_scalar(condition, inputs)?) {
                    result = eval_scalar(value, inputs)?;
                    matched = true;
                    break;
                }
            }
            if !matched {
                if let Some(value) = else_expr {
                    result = eval_scalar(value, inputs)?;
                }
            }
            result
        }
        Expr::TextLength(_) | Expr::TextCompare { .. } | Expr::TextLike { .. } => {
            return Err("scalar derived expressions must be numeric".into());
        }
    };
    Ok(value)
}

fn eval_output(
    expression: &OutputExpression,
    inputs: &HashMap<String, Option<String>>,
) -> Result<Option<String>, String> {
    let value = match expression {
        OutputExpression::Scalar(expression) => eval_scalar(expression, inputs)?,
        OutputExpression::NullWhen {
            condition,
            otherwise,
        } => {
            if truthy(eval_scalar(condition, inputs)?) {
                EvalValue::Null
            } else {
                eval_scalar(otherwise, inputs)?
            }
        }
    };
    match value {
        EvalValue::Null => Ok(None),
        EvalValue::Number(value) if value.is_finite() => Ok(Some(value.to_string())),
        EvalValue::Number(_) => Err("scalar derived expression returned a non-finite value".into()),
        EvalValue::Boolean(value) => Ok(Some(if value { "1" } else { "0" }.into())),
    }
}

pub(super) fn materialize(
    database: &Path,
    plan: &ScalarDerivedCross,
) -> Result<TemporaryPart, String> {
    let mut input_names = Vec::new();
    let mut input_values = HashMap::new();
    let mut seen = HashSet::new();
    for query in &plan.queries {
        let output = execute_inner_query(query, database)?;
        let (names, row) = decode_single_row(&output)?;
        if names.len() > MAX_DERIVED_COLUMNS.saturating_sub(input_names.len()) {
            return Err("scalar derived query supports at most 128 output columns".into());
        }
        for (name, value) in names.into_iter().zip(row) {
            if name.is_empty() || name.contains(['\t', '\n', '\r']) {
                return Err("scalar derived output has an invalid column name".into());
            }
            if !seen.insert(name.to_ascii_lowercase()) {
                return Err("scalar derived output column names must be unique".into());
            }
            input_names.push(name.clone());
            input_values.insert(name.to_ascii_lowercase(), value);
        }
    }
    let (names, values) = if let Some(outputs) = &plan.outputs {
        let names = outputs
            .iter()
            .map(|output| output.name.clone())
            .collect::<Vec<_>>();
        let mut seen = HashSet::new();
        if names.iter().any(|name| {
            name.is_empty()
                || name.contains(['\t', '\n', '\r'])
                || !seen.insert(name.to_ascii_lowercase())
        }) {
            return Err("scalar derived output column names must be unique".into());
        }
        let values = outputs
            .iter()
            .map(|output| eval_output(&output.expression, &input_values))
            .collect::<Result<Vec<_>, _>>()?;
        (names, values)
    } else {
        let values = input_names
            .iter()
            .map(|name| {
                input_values
                    .get(&name.to_ascii_lowercase())
                    .cloned()
                    .flatten()
            })
            .collect();
        (input_names, values)
    };
    if names.is_empty() || names.len() > MAX_DERIVED_COLUMNS {
        return Err("scalar derived query supports 1 to 128 output columns".into());
    }
    let mut output_rename = None;
    let columns = names
        .into_iter()
        .zip(&values)
        .enumerate()
        .map(|(index, (name, value))| {
            let physical_name = if valid_alias(&name) && name.len() <= 128 {
                name
            } else if output_rename.is_none() {
                let physical_name = format!("scalar_output_{index}");
                output_rename = Some((physical_name.clone(), name));
                physical_name
            } else {
                String::new()
            };
            Column {
                name: physical_name,
                data_type: if value.is_some() {
                    "String"
                } else {
                    "Nullable(String)"
                }
                .into(),
            }
        })
        .collect::<Vec<_>>();
    if columns.iter().any(|column| column.name.is_empty()) {
        return Err("only one scalar derived output name can require quoting".into());
    }
    let retained_bytes = values.iter().flatten().map(String::len).sum::<usize>();
    if retained_bytes > MAX_DERIVED_BYTES {
        return Err("scalar derived result exceeds 8 MiB".into());
    }
    let root = allocate_temp_root()?;
    let path = root.join("part");
    let temporary = TemporaryPart {
        root,
        path,
        output_rename,
    };
    write_wide_part(&temporary.path, &columns, &[values]).map_err(|error| error.to_string())?;
    Ok(temporary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_multiple_scalar_subqueries_and_outer_limit() {
        let plan = parse(
            "SELECT * FROM (SELECT count(*) AS a FROM first) x, (SELECT avg(value) b FROM second) y LIMIT 1",
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.queries.len(), 2);
        assert_eq!(plan.queries[0], "SELECT count(*) AS a FROM first");
        assert_eq!(plan.limit, Some(1));
    }

    #[test]
    fn parses_scalar_ratio_projection_and_cast_aliases() {
        let plan = parse(
            "SELECT promotions, total, cast(promotions AS decimal(15,4))/cast(total AS decimal(15,4))*100 FROM (SELECT sum(price) promotions FROM sales) s, (SELECT sum(price) total FROM sales) t ORDER BY promotions, total LIMIT 100",
        )
        .unwrap()
        .unwrap();
        let outputs = plan.outputs.unwrap();
        assert_eq!(outputs.len(), 3);
        assert_eq!(outputs[0].name, "promotions");
        assert_eq!(outputs[1].name, "total");
        assert_eq!(
            outputs[2].name,
            "((CAST(promotions AS DECIMAL(15, 4)) / CAST(total AS DECIMAL(15, 4))) * 100)"
        );
    }

    #[test]
    fn evaluates_case_null_and_numeric_ratio_outputs() {
        let plan = parse(
            "SELECT case when pmc=0 then null else cast(amc AS decimal(15,4))/cast(pmc AS decimal(15,4)) end am_pm_ratio FROM (SELECT count(*) amc FROM sales) a, (SELECT count(*) pmc FROM sales) p ORDER BY am_pm_ratio LIMIT 100",
        )
        .unwrap()
        .unwrap();
        let output = plan.outputs.unwrap().remove(0);
        let mut inputs = HashMap::from([
            ("amc".into(), Some("3".into())),
            ("pmc".into(), Some("0".into())),
        ]);
        assert_eq!(eval_output(&output.expression, &inputs).unwrap(), None);
        inputs.insert("pmc".into(), Some("2".into()));
        assert_eq!(
            eval_output(&output.expression, &inputs).unwrap(),
            Some("1.5".into())
        );
    }

    #[test]
    fn ignores_a_leading_benchmark_comment() {
        let plan = parse("-- benchmark query\nSELECT * FROM (SELECT count(*) AS a FROM first) x")
            .unwrap()
            .unwrap();
        assert_eq!(plan.queries.len(), 1);
    }

    #[test]
    fn ignores_commas_and_parentheses_inside_sql_strings() {
        let plan = parse(
            "SELECT * FROM (SELECT count(*) AS a FROM first WHERE label = 'x, (y)') x, (SELECT count(*) b FROM second) y",
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.queries.len(), 2);
    }

    #[test]
    fn leaves_non_scalar_derived_queries_to_other_parsers() {
        for query in [
            "SELECT * FROM (SELECT a FROM first) x",
            "SELECT a FROM (SELECT a FROM first) x",
            "SELECT * FROM first, second",
        ] {
            assert!(parse(query).unwrap().is_none(), "{query}");
        }
    }
}
