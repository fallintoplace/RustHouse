//! Bounded EXCEPT DISTINCT over simple SELECT projections.
//! Keep the set operation after each branch's normal predicate evaluation.

use crate::lexer::{Token, tokenize};
use crate::{ExecutionError, ParseError, Projection, Select, Source, execute, parse_select_tokens};
use rusthouse_storage::inspect_query_source;
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const MAX_OPERANDS: usize = 3;
const MAX_SET_ROWS: usize = 32_768;
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

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
    if plan.selects.len() < 2 || plan.selects.len() > MAX_OPERANDS
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
