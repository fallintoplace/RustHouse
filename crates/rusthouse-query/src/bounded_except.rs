//! Bounded EXCEPT DISTINCT over simple SELECT projections.
//! Keep the set operation after each branch's normal predicate evaluation.

mod set_rewrite;
use self::set_rewrite::*;

use crate::lexer::{Token, tokenize};
use crate::{
    DerivedColumn, ExecutionError, JoinSelect, ParseError, Projection, Select, Source, execute,
    execute_multi_join, parse_join_with_schema, parse_select_tokens,
};
use rusthouse_storage::inspect_query_source;
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const MAX_OPERANDS: usize = 3;
const MAX_SET_ROWS: usize = 32_768;
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivedSetOperation {
    Except,
    Intersect,
}

#[derive(Debug, Clone)]
pub struct DerivedSetCount {
    branches: Vec<JoinSelect>,
    operation: DerivedSetOperation,
    output_name: String,
    width: usize,
    limit: Option<usize>,
    offset: usize,
}

impl DerivedSetCount {
    pub fn branches(&self) -> &[JoinSelect] {
        &self.branches
    }

    pub fn operation(&self) -> DerivedSetOperation {
        self.operation
    }
}

#[derive(Debug, Clone)]
pub struct ExceptSelect {
    pub selects: Vec<Select>,
}

pub fn parse_bounded_except(sql: &str) -> Result<Option<ExceptSelect>, ParseError> {
    let tokens = tokenize(sql)?;
    let mut depth = 0usize;
    let mut bounds = vec![0usize];
    let mut index = 0usize;
    while index < tokens.len() {
        match &tokens[index] {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(name) if depth == 0 && name.eq_ignore_ascii_case("EXCEPT") => {
                if bounds.len() == MAX_OPERANDS {
                    return Err(ParseError("EXCEPT supports at most 3 SELECTs".into()));
                }
                if matches!(tokens.get(index + 1), Some(Token::Word(next))
                    if next.eq_ignore_ascii_case("ALL"))
                {
                    return Err(ParseError("EXCEPT ALL is not supported".into()));
                }
                bounds.push(index + 1);
            }
            _ => {}
        }
        index += 1;
    }
    if bounds.len() == 1 {
        return Ok(None);
    }
    bounds.push(tokens.len() + 1);
    let mut selects = Vec::with_capacity(bounds.len() - 1);
    for pair in bounds.windows(2) {
        let start = pair[0];
        let end = pair[1] - 1;
        let mut segment = tokens[start..end].to_vec();
        if matches!(segment.first(), Some(Token::Word(name)) if name.eq_ignore_ascii_case("DISTINCT"))
        {
            segment.remove(0);
        }
        let select = parse_select_tokens(segment)?;
        if !matches!(select.source, Some(Source::Table(_)))
            || !matches!(&select.projection, Projection::Columns(cols) if (1..=8).contains(&cols.len()))
            || select.order_by.is_some()
            || !select.order_by_extra.is_empty()
            || select.limit.is_some()
            || select.offset.is_some()
        {
            return Err(ParseError(
                "EXCEPT requires simple table projections of 1 to 8 columns without ORDER or LIMIT"
                    .into(),
            ));
        }
        selects.push(select);
    }
    Ok(Some(ExceptSelect { selects }))
}

/// Parse COUNT(*) over a bounded EXCEPT or INTERSECT between joined SELECTs.
pub fn parse_derived_set_count_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<DerivedColumn>, ParseError>,
) -> Result<Option<DerivedSetCount>, ParseError> {
    let tokens = tokenize(sql)?;
    let mut parser = crate::Parser { tokens, index: 0 };
    if !parser.keyword("SELECT") || !parser.keyword("COUNT") {
        return Ok(None);
    }
    if !parser.take(Token::LParen) || !parser.take(Token::Star) || !parser.take(Token::RParen) {
        return Ok(None);
    }
    let output_name = if parser.keyword("AS") {
        parser.bounded_word()?
    } else if matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if !word.eq_ignore_ascii_case("FROM"))
    {
        parser.bounded_word()?
    } else {
        "count_star()".into()
    };
    if !parser.keyword("FROM") || !parser.take(Token::LParen) {
        return Ok(None);
    }
    let open = parser.index - 1;
    let mut depth = 1usize;
    while parser.index < parser.tokens.len() && depth > 0 {
        match parser.tokens[parser.index] {
            Token::LParen => depth += 1,
            Token::RParen => depth -= 1,
            _ => {}
        }
        if depth > 0 {
            parser.index += 1;
        }
    }
    if depth != 0 {
        return Err(ParseError("derived set count requires closing ')'".into()));
    }
    let close = parser.index;
    let Some((operation, branch_tokens)) = split_derived_set(&parser.tokens[open + 1..close])?
    else {
        return Ok(None);
    };
    parser.index += 1;
    if parser.keyword("AS") {
        parser.bounded_word()?;
    } else if matches!(parser.tokens.get(parser.index), Some(Token::Word(word) | Token::QuotedWord(word) | Token::QuotedAlias(word)) if !["LIMIT", "OFFSET", "FORMAT"].iter().any(|keyword| word.eq_ignore_ascii_case(keyword)))
    {
        parser.index += 1;
    }
    let mut limit = None;
    let mut offset = 0;
    if parser.keyword("LIMIT") {
        let value = parser.number()?;
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError("LIMIT requires a nonnegative integer".into()));
        }
        limit = Some(
            value
                .parse()
                .map_err(|_| ParseError("LIMIT is too large".into()))?,
        );
    }
    if parser.keyword("OFFSET") {
        let value = parser.number()?;
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError("OFFSET requires a nonnegative integer".into()));
        }
        offset = value
            .parse()
            .map_err(|_| ParseError("OFFSET is too large".into()))?;
    }
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError("unsupported derived set count suffix".into()));
    }

    let mut branches = Vec::with_capacity(branch_tokens.len());
    let mut expected_types: Option<Vec<String>> = None;
    for tokens in branch_tokens {
        let branch_sql = render_set_tokens(&tokens);
        let Some(branch) = parse_join_with_schema(&branch_sql, |table| {
            resolve(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
        })?
        else {
            return Ok(None);
        };
        if branch.kind != crate::JoinType::Inner
            || branch
                .steps
                .iter()
                .any(|step| step.kind != crate::JoinType::Inner)
            || branch.columns.is_none()
            || branch.aggregate.is_some()
            || !branch.scalar_aggregates.is_empty()
            || branch.grouped_count.is_some()
            || branch.grouped_keys_only
            || branch.grouped_sum.is_some()
            || !branch.grouped_case_counts.is_empty()
            || branch.scalar_sum.is_some()
            || branch.scalar_formula.is_some()
            || branch.limit.is_some()
            || branch.offset != 0
            || !branch.order_by.is_empty()
        {
            return Ok(None);
        }
        let Some(types) = projection_types(&branch, &mut resolve)? else {
            return Ok(None);
        };
        if expected_types
            .as_ref()
            .is_some_and(|expected| expected != &types)
        {
            return Ok(None);
        }
        expected_types.get_or_insert(types);
        branches.push(branch);
    }
    let width = expected_types
        .expect("at least two valid set branches")
        .len();
    Ok(Some(DerivedSetCount {
        branches,
        operation,
        output_name,
        width,
        limit,
        offset,
    }))
}

struct BoundedOutput {
    bytes: Vec<u8>,
}

impl BoundedOutput {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }
}

impl Write for BoundedOutput {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(buf.len())
            .is_none_or(|total| total > MAX_RESULT_BYTES)
        {
            return Err(io::Error::other("EXCEPT input exceeds 8 MiB"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn invalid(message: &str) -> ExecutionError {
    ExecutionError::Invalid(message.into())
}

fn projected_types(select: &Select, path: &Path) -> Result<Vec<String>, ExecutionError> {
    let Projection::Columns(columns) = &select.projection else {
        return Err(invalid("EXCEPT requires column projections"));
    };
    let metadata = inspect_query_source(path)?;
    columns
        .iter()
        .map(|name| {
            metadata
                .columns
                .iter()
                .find(|column| &column.name == name)
                .map(|column| column.data_type.clone())
                .ok_or_else(|| invalid("unknown EXCEPT projection column"))
        })
        .collect()
}

/// Evaluate each branch under its own SELECT predicates. EXCEPT is DISTINCT
/// and compares positions, requiring identical physical column types.
pub fn execute_bounded_except<W: Write>(
    plan: &ExceptSelect,
    paths: &[PathBuf],
    output: &mut W,
) -> Result<(), ExecutionError> {
    if plan.selects.len() < 2
        || plan.selects.len() > MAX_OPERANDS
        || paths.len() != plan.selects.len()
    {
        return Err(invalid("EXCEPT requires 2 or 3 matching inputs"));
    }
    let mut expected_types = None;
    let mut first_header = Vec::new();
    let mut rows = HashSet::<Vec<u8>>::new();
    let mut retained = 0usize;
    for (position, (select, path)) in plan.selects.iter().zip(paths).enumerate() {
        let types = projected_types(select, path)?;
        if let Some(expected) = &expected_types {
            if expected != &types {
                return Err(invalid("EXCEPT projections require matching column types"));
            }
        } else {
            expected_types = Some(types);
        }
        let mut bound = select.clone();
        bound.source = Some(Source::Table("part".into()));
        let mut buffer = BoundedOutput::new();
        execute(&bound, Some(path), &mut buffer)?;
        let Some(header_end) = buffer.bytes.iter().position(|byte| *byte == b'\n') else {
            return Err(invalid("EXCEPT SELECT omitted output header"));
        };
        let header = &buffer.bytes[..header_end];
        let width = header.iter().filter(|&&byte| byte == b'\t').count() + 1;
        if position == 0 {
            first_header.extend_from_slice(header);
        }
        if width != expected_types.as_ref().expect("types checked").len() {
            return Err(invalid("EXCEPT result width does not match projection"));
        }
        let body = &buffer.bytes[header_end + 1..];
        if !body.is_empty() {
            if body.last() != Some(&b'\n') {
                return Err(invalid("EXCEPT SELECT output is not line terminated"));
            }
            for row in body[..body.len() - 1].split(|byte| *byte == b'\n') {
                if row.iter().filter(|&&byte| byte == b'\t').count() + 1 != width {
                    return Err(invalid("EXCEPT row width does not match SELECT columns"));
                }
                if position == 0 {
                    if !rows.contains(row) {
                        if rows.len() >= MAX_SET_ROWS {
                            return Err(invalid("EXCEPT supports at most 32768 distinct rows"));
                        }
                        retained = retained
                            .checked_add(row.len() + std::mem::size_of::<Vec<u8>>() + 64)
                            .filter(|&total| total <= MAX_RESULT_BYTES)
                            .ok_or_else(|| invalid("EXCEPT set exceeds 8 MiB"))?;
                        rows.insert(row.to_vec());
                    }
                } else {
                    rows.remove(row);
                }
            }
        }
    }
    output.write_all(&first_header)?;
    output.write_all(b"\n")?;
    let mut sorted = rows.into_iter().collect::<Vec<_>>();
    sorted.sort_unstable();
    for row in sorted {
        output.write_all(&row)?;
        output.write_all(b"\n")?;
    }
    Ok(())
}

/// Evaluate bounded joined branches, apply EXCEPT/INTERSECT DISTINCT, and
/// count the resulting rows.
pub fn execute_derived_set_count<W: Write>(
    plan: &DerivedSetCount,
    paths: &[Vec<PathBuf>],
    output: &mut W,
) -> Result<(), ExecutionError> {
    if plan.branches.len() < 2
        || plan.branches.len() > MAX_OPERANDS
        || paths.len() != plan.branches.len()
    {
        return Err(invalid("derived set count requires 2 or 3 matching inputs"));
    }

    let mut rows = HashSet::<Vec<u8>>::new();
    for (branch_index, (branch, branch_paths)) in plan.branches.iter().zip(paths).enumerate() {
        let mut buffer = BoundedOutput::new();
        execute_multi_join(branch, branch_paths, &mut buffer)?;
        let Some(header_end) = buffer.bytes.iter().position(|byte| *byte == b'\n') else {
            return Err(invalid("derived set branch omitted its output header"));
        };
        let header_width = buffer.bytes[..header_end]
            .iter()
            .filter(|&&byte| byte == b'\t')
            .count()
            + 1;
        if header_width != plan.width {
            return Err(invalid("derived set branch output width does not match"));
        }
        let body = &buffer.bytes[header_end + 1..];
        if !body.is_empty() && body.last() != Some(&b'\n') {
            return Err(invalid("derived set branch output is not line terminated"));
        }

        if branch_index == 0 {
            let mut retained = 0usize;
            if !body.is_empty() {
                for row in body
                    .strip_suffix(b"\n")
                    .unwrap_or_default()
                    .split(|byte| *byte == b'\n')
                {
                    insert_bounded_set_row(&mut rows, &mut retained, row, plan.width)?;
                }
            }
            continue;
        }

        let mut branch_rows = HashSet::<Vec<u8>>::new();
        let mut branch_retained = 0usize;
        if !body.is_empty() {
            for row in body
                .strip_suffix(b"\n")
                .unwrap_or_default()
                .split(|byte| *byte == b'\n')
            {
                insert_bounded_set_row(&mut branch_rows, &mut branch_retained, row, plan.width)?;
            }
        }
        match plan.operation {
            DerivedSetOperation::Except => rows.retain(|row| !branch_rows.contains(row)),
            DerivedSetOperation::Intersect => rows.retain(|row| branch_rows.contains(row)),
        }
    }

    writeln!(output, "{}", plan.output_name)?;
    if plan.limit == Some(0) || plan.offset > 0 {
        return Ok(());
    }
    writeln!(output, "{}", rows.len())?;
    Ok(())
}

fn insert_bounded_set_row(
    rows: &mut HashSet<Vec<u8>>,
    retained: &mut usize,
    row: &[u8],
    width: usize,
) -> Result<(), ExecutionError> {
    if row.iter().filter(|&&byte| byte == b'\t').count() + 1 != width {
        return Err(invalid("derived set row width does not match projection"));
    }
    if rows.contains(row) {
        return Ok(());
    }
    if rows.len() >= MAX_SET_ROWS {
        return Err(invalid("derived set supports at most 32768 distinct rows"));
    }
    retained
        .checked_add(row.len() + std::mem::size_of::<Vec<u8>>() + 64)
        .filter(|&total| total <= MAX_RESULT_BYTES)
        .ok_or_else(|| invalid("derived set exceeds 8 MiB"))?;
    rows.insert(row.to_vec());
    *retained += row.len() + std::mem::size_of::<Vec<u8>>() + 64;
    Ok(())
}
