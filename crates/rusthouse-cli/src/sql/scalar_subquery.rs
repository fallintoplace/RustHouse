//! Rewrite bounded, uncorrelated scalar SELECTs into SQL literals.
//!
//! This keeps simple scalar aggregates and one-row lookups on the normal
//! SELECT path while leaving correlated and set-returning subqueries alone.

use rusthouse_query::{
    Projection, ScalarExpr, Select, Source, parse_scalar_expression, parse_select,
};
use std::io::{self, Write};
use std::path::Path;

const MAX_SCALAR_SUBQUERIES: usize = 64;
const MAX_SCALAR_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
struct BoundedOutput {
    bytes: Vec<u8>,
}

impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > MAX_SCALAR_OUTPUT_BYTES {
            return Err(io::Error::other("scalar subquery output exceeds 8 MiB"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$')
}

fn keyword_at(sql: &str, index: usize, keyword: &str) -> bool {
    let bytes = sql.as_bytes();
    let end = index.saturating_add(keyword.len());
    end <= bytes.len()
        && bytes[index..end].eq_ignore_ascii_case(keyword.as_bytes())
        && (index == 0 || !identifier_byte(bytes[index - 1]))
        && (end == bytes.len() || !identifier_byte(bytes[end]))
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

fn skip_comment(sql: &str, index: usize) -> usize {
    let bytes = sql.as_bytes();
    if bytes.get(index) == Some(&b'#') {
        return sql[index + 1..]
            .find(['\r', '\n'])
            .map_or(bytes.len(), |offset| index + 1 + offset);
    }
    if bytes.get(index..index + 2) == Some(b"--") {
        return sql[index + 2..]
            .find(['\r', '\n'])
            .map_or(bytes.len(), |offset| index + 2 + offset);
    }
    if bytes.get(index..index + 2) == Some(b"/*") {
        return sql[index + 2..]
            .find("*/")
            .map_or(bytes.len(), |offset| index + 2 + offset + 2);
    }
    index
}

fn leading_select_start(sql: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut cursor = 0usize;
    loop {
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let next = skip_comment(sql, cursor);
        if next == cursor {
            break;
        }
        cursor = next;
    }
    keyword_at(sql, cursor, "SELECT").then_some(cursor)
}

pub(super) fn starts_with_select(sql: &str) -> bool {
    leading_select_start(sql).is_some()
}

fn matching_close(sql: &str, open: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut cursor = open;
    let mut depth = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            b'-' | b'/' if skip_comment(sql, cursor) != cursor => {
                cursor = skip_comment(sql, cursor)
            }
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Clause {
    Select,
    From,
    Other,
}

#[derive(Debug, Clone)]
struct CorrelatedCountSpan {
    open: usize,
    end: usize,
    body_start: usize,
    body_end: usize,
    key: String,
}

#[derive(Debug, Clone)]
struct QualifiedIdentifier {
    start: usize,
    end: usize,
    relation: String,
    column: String,
}

fn clause_at(sql: &str, end: usize) -> Option<Clause> {
    let bytes = sql.as_bytes();
    let mut clauses = vec![None];
    let mut cursor = 0usize;
    while cursor < end {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            b'-' | b'/' if skip_comment(sql, cursor) != cursor => {
                cursor = skip_comment(sql, cursor)
            }
            b'(' => {
                clauses.push(None);
                cursor += 1;
            }
            b')' => {
                clauses.pop();
                cursor += 1;
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                let start = cursor;
                cursor += 1;
                while cursor < end && identifier_byte(bytes[cursor]) {
                    cursor += 1;
                }
                let clause = if sql[start..cursor].eq_ignore_ascii_case("SELECT") {
                    Some(Clause::Select)
                } else if sql[start..cursor].eq_ignore_ascii_case("FROM") {
                    Some(Clause::From)
                } else if ["WHERE", "GROUP", "ORDER", "HAVING", "LIMIT", "QUALIFY"]
                    .iter()
                    .any(|word| sql[start..cursor].eq_ignore_ascii_case(word))
                {
                    Some(Clause::Other)
                } else {
                    None
                };
                if let Some(clause) = clause {
                    if let Some(current) = clauses.last_mut() {
                        *current = Some(clause);
                    }
                }
            }
            _ => cursor += 1,
        }
    }
    clauses.last().copied().flatten()
}

fn previous_word(sql: &str, index: usize) -> Option<&str> {
    let bytes = sql.as_bytes();
    let mut end = index;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && identifier_byte(bytes[start - 1]) {
        start -= 1;
    }
    (start != end).then(|| &sql[start..end])
}

fn has_qualified_column(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            b'-' | b'/' if skip_comment(sql, cursor) != cursor => {
                cursor = skip_comment(sql, cursor)
            }
            b'.' if cursor > 0 && cursor + 1 < bytes.len() => {
                let mut left_start = cursor;
                while left_start > 0 && identifier_byte(bytes[left_start - 1]) {
                    left_start -= 1;
                }
                let left_is_identifier = left_start < cursor
                    && bytes[left_start].is_ascii_alphabetic()
                    || left_start < cursor && matches!(bytes[left_start], b'_' | b'$');
                let right_is_identifier = bytes[cursor + 1].is_ascii_alphabetic()
                    || matches!(bytes[cursor + 1], b'_' | b'$' | b'"' | b'`');
                if left_is_identifier && right_is_identifier {
                    return true;
                }
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    false
}

fn source_subquery(sql: &str, open: usize) -> bool {
    let previous = previous_word(sql, open);
    if previous.is_some_and(|word| {
        ["FROM", "JOIN", "EXISTS", "IN", "ANY", "ALL"]
            .iter()
            .any(|keyword| word.eq_ignore_ascii_case(keyword))
    }) {
        return true;
    }
    let mut cursor = open;
    while cursor > 0 && sql.as_bytes()[cursor - 1].is_ascii_whitespace() {
        cursor -= 1;
    }
    cursor > 0 && sql.as_bytes()[cursor - 1] == b',' && clause_at(sql, open) == Some(Clause::From)
}

fn identifier_end(bytes: &[u8], start: usize) -> Option<usize> {
    let first = *bytes.get(start)?;
    if !(first.is_ascii_alphabetic() || matches!(first, b'_' | b'$')) {
        return None;
    }
    let mut end = start + 1;
    while bytes.get(end).is_some_and(|byte| identifier_byte(*byte)) {
        end += 1;
    }
    Some(end)
}

fn qualified_identifiers(sql: &str) -> Vec<QualifiedIdentifier> {
    let bytes = sql.as_bytes();
    let mut identifiers = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            b'-' | b'/' if skip_comment(sql, cursor) != cursor => {
                cursor = skip_comment(sql, cursor)
            }
            byte if byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$') => {
                let start = cursor;
                let Some(left_end) = identifier_end(bytes, cursor) else {
                    cursor += 1;
                    continue;
                };
                let mut dot = left_end;
                while bytes.get(dot).is_some_and(u8::is_ascii_whitespace) {
                    dot += 1;
                }
                if bytes.get(dot) != Some(&b'.') {
                    cursor = left_end;
                    continue;
                }
                let mut right_start = dot + 1;
                while bytes.get(right_start).is_some_and(u8::is_ascii_whitespace) {
                    right_start += 1;
                }
                if let Some(right_end) = identifier_end(bytes, right_start) {
                    identifiers.push(QualifiedIdentifier {
                        start,
                        end: right_end,
                        relation: sql[start..left_end].to_owned(),
                        column: sql[right_start..right_end].to_owned(),
                    });
                    cursor = right_end;
                } else {
                    cursor = dot + 1;
                }
            }
            _ => cursor += 1,
        }
    }
    identifiers
}

fn replace_qualified_identifiers(
    sql: &str,
    identifiers: &[QualifiedIdentifier],
    value: &str,
) -> String {
    let mut output = String::with_capacity(sql.len());
    let mut copied = 0usize;
    for identifier in identifiers {
        output.push_str(&sql[copied..identifier.start]);
        output.push_str(value);
        copied = identifier.end;
    }
    output.push_str(&sql[copied..]);
    output
}

fn replace_span(sql: &str, start: usize, end: usize, value: &str) -> String {
    let mut output = String::with_capacity(sql.len() + value.len());
    output.push_str(&sql[..start]);
    output.push_str(value);
    output.push_str(&sql[end..]);
    output
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

fn correlated_count_span(sql: &str) -> Option<CorrelatedCountSpan> {
    let bytes = sql.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            b'-' | b'/' if skip_comment(sql, cursor) != cursor => {
                cursor = skip_comment(sql, cursor)
            }
            b'(' => {
                let Some(close) = matching_close(sql, cursor) else {
                    return None;
                };
                let body = &sql[cursor + 1..close];
                let identifiers = qualified_identifiers(body);
                if body
                    .trim_start()
                    .get(..6)
                    .is_some_and(|word| word.eq_ignore_ascii_case("SELECT"))
                    && !identifiers.is_empty()
                {
                    let relation = &identifiers[0].relation;
                    let column = &identifiers[0].column;
                    if identifiers.iter().all(|identifier| {
                        identifier.relation.eq_ignore_ascii_case(relation)
                            && identifier.column.eq_ignore_ascii_case(column)
                    }) {
                        let uncorrelated = replace_qualified_identifiers(body, &identifiers, "0");
                        let select = match parse_select(&uncorrelated) {
                            Ok(select) => select,
                            Err(_) => {
                                cursor += 1;
                                continue;
                            }
                        };
                        if matches!(select.projection, Projection::Count)
                            && matches!(select.source, Some(Source::Table(_)))
                        {
                            let mut end = skip_space(sql, close + 1);
                            if bytes.get(end) != Some(&b'>') || bytes.get(end + 1) == Some(&b'=') {
                                cursor += 1;
                                continue;
                            }
                            end = skip_space(sql, end + 1);
                            if bytes.get(end) != Some(&b'0')
                                || bytes
                                    .get(end + 1)
                                    .is_some_and(|byte| identifier_byte(*byte))
                            {
                                cursor += 1;
                                continue;
                            }
                            return Some(CorrelatedCountSpan {
                                open: cursor,
                                end: end + 1,
                                body_start: cursor + 1,
                                body_end: close,
                                key: column.clone(),
                            });
                        }
                    }
                }
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    None
}

fn first_top_level_keyword(sql: &str, words: &[&str], start: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut cursor = start;
    let mut depth = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            b'-' | b'/' if skip_comment(sql, cursor) != cursor => {
                cursor = skip_comment(sql, cursor)
            }
            b'(' => {
                depth += 1;
                cursor += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                cursor += 1;
            }
            byte if depth == 0 && (byte.is_ascii_alphabetic() || byte == b'_') => {
                let start = cursor;
                cursor += 1;
                while cursor < bytes.len() && identifier_byte(bytes[cursor]) {
                    cursor += 1;
                }
                if words
                    .iter()
                    .any(|word| sql[start..cursor].eq_ignore_ascii_case(word))
                {
                    return Some(start);
                }
            }
            _ => cursor += 1,
        }
    }
    None
}

fn decode_distinct_values(output: &[u8]) -> Option<Vec<String>> {
    if output.len() > MAX_SCALAR_OUTPUT_BYTES {
        return None;
    }
    let text = std::str::from_utf8(output).ok()?;
    let mut lines = text.split_terminator('\n');
    let header = lines.next()?;
    if header.contains('\t') {
        return None;
    }
    lines
        .map(|line| {
            if line == r"\N" {
                return None;
            }
            let decoded = crate::tsv_escape::decode_tsv_field(line.as_bytes()).ok()?;
            String::from_utf8(decoded).ok()
        })
        .collect()
}

fn literal_for_type(value: &str, data_type: &str) -> String {
    let base_type = data_type
        .strip_prefix("Nullable(")
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(data_type);
    if base_type.contains("String") || base_type.starts_with("FixedString") {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        sql_literal(Some(value))
    }
}

fn correlated_count_candidate_query(query: &str, span: &CorrelatedCountSpan) -> Option<String> {
    let from = first_top_level_keyword(query, &["FROM"], 0)?;
    let end = first_top_level_keyword(query, &["ORDER", "LIMIT", "OFFSET", "FORMAT"], from)
        .unwrap_or(query.len());
    Some(format!(
        "SELECT DISTINCT {} {}{} IS NOT NULL{}",
        span.key,
        &query[from..span.open],
        span.key,
        &query[span.end..end]
    ))
}

fn correlated_count_filter(key: &str, eligible: &[String]) -> String {
    if eligible.is_empty() {
        format!("{key} IS NULL AND {key} IS NOT NULL")
    } else {
        format!("{key} IN ({})", eligible.join(", "))
    }
}

fn rewrite_correlated_count_positive(query: &str, database: &Path) -> Option<String> {
    let span = correlated_count_span(query)?;
    let candidate_query = correlated_count_candidate_query(query, &span)?;
    let mut candidate = parse_select(&candidate_query).ok()?;
    let Some(Source::Table(outer_table)) = candidate.source.clone() else {
        return None;
    };
    let outer_path = crate::database::select_source(database, &outer_table).ok()?;
    candidate.source = Some(Source::Table("part".into()));
    let mut candidate_output = BoundedOutput::default();
    if rusthouse_query::execute(&candidate, Some(&outer_path), &mut candidate_output).is_err() {
        return None;
    }
    let values = match decode_distinct_values(&candidate_output.bytes) {
        Some(values) => values,
        None => return None,
    };
    if values.len() > 64 {
        return None;
    }
    let key_type = crate::catalog::read_schema(database, &outer_table)
        .ok()?
        .into_iter()
        .find(|column| column.name.eq_ignore_ascii_case(&span.key))?
        .data_type;

    let inner = &query[span.body_start..span.body_end];
    let identifiers = qualified_identifiers(inner);
    let mut eligible = Vec::new();
    for value in values {
        let literal = literal_for_type(&value, &key_type);
        let inner_query = replace_qualified_identifiers(inner, &identifiers, &literal);
        let mut inner_select = parse_select(&inner_query).ok()?;
        let Some(Source::Table(inner_table)) = inner_select.source.clone() else {
            return None;
        };
        let inner_path = crate::database::select_source(database, &inner_table).ok()?;
        inner_select.source = Some(Source::Table("part".into()));
        let mut count_output = BoundedOutput::default();
        if rusthouse_query::execute(&inner_select, Some(&inner_path), &mut count_output).is_err() {
            return None;
        }
        let count = decode_single_value(&count_output.bytes)?
            .and_then(|value| value.parse::<u64>().ok())?;
        if count > 0 {
            eligible.push(literal);
        }
    }
    let replacement = correlated_count_filter(&span.key, &eligible);
    Some(replace_span(query, span.open, span.end, &replacement))
}

fn normalize_distinct_parentheses(query: &str) -> String {
    let Some(select_start) = leading_select_start(query) else {
        return query.to_owned();
    };
    let prefix = &query[..select_start];
    let query = &query[select_start..];
    let mut projection_start = "SELECT".len();
    while query
        .as_bytes()
        .get(projection_start)
        .is_some_and(u8::is_ascii_whitespace)
    {
        projection_start += 1;
    }
    if !keyword_at(query, projection_start, "DISTINCT") {
        return format!("{prefix}{query}");
    }
    projection_start += "DISTINCT".len();
    while query
        .as_bytes()
        .get(projection_start)
        .is_some_and(u8::is_ascii_whitespace)
    {
        projection_start += 1;
    }
    if query.as_bytes().get(projection_start) != Some(&b'(') {
        return format!("{prefix}{query}");
    }
    let Some(close) = matching_close(query, projection_start) else {
        return format!("{prefix}{query}");
    };
    let value = query[projection_start + 1..close].trim();
    if value.is_empty()
        || value
            .bytes()
            .any(|byte| !(identifier_byte(byte) || byte == b'.'))
    {
        return format!("{prefix}{query}");
    }
    format!(
        "{prefix}{} {}{}",
        query[..projection_start].trim_end(),
        value,
        &query[close + 1..]
    )
}

fn simple_scalar_projection(select: &Select) -> bool {
    match &select.projection {
        Projection::Count
        | Projection::CountColumn { .. }
        | Projection::CountDistinct { .. }
        | Projection::Aggregate { .. } => true,
        Projection::MultipleAggregates(items) => items.len() == 1,
        Projection::Columns(columns) => select.distinct && columns.len() == 1,
        _ => false,
    }
}

fn decode_single_value(output: &[u8]) -> Option<Option<String>> {
    if output.len() > MAX_SCALAR_OUTPUT_BYTES {
        return None;
    }
    let text = std::str::from_utf8(output).ok()?;
    let mut lines = text.split_terminator('\n');
    let header = lines.next()?;
    if header.contains('\t') {
        return None;
    }
    let Some(row) = lines.next() else {
        return Some(None);
    };
    if lines.next().is_some() || row.contains('\t') {
        return None;
    }
    if row == r"\N" {
        return Some(None);
    }
    let decoded = crate::tsv_escape::decode_tsv_field(row.as_bytes()).ok()?;
    Some(Some(String::from_utf8(decoded).ok()?))
}

fn sql_literal(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "NULL".into();
    };
    if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false") {
        return value.to_ascii_uppercase();
    }
    if value.parse::<f64>().is_ok_and(f64::is_finite) {
        return value.to_owned();
    }
    format!("'{}'", value.replace('\'', "''"))
}

fn evaluate(query: &str, database: &Path) -> Option<Option<String>> {
    if has_qualified_column(query) {
        return None;
    }
    let query = normalize_distinct_parentheses(query.trim());
    let mut select = match parse_select(&query) {
        Ok(select) => select,
        Err(_) => return None,
    };
    if !simple_scalar_projection(&select) || select.having.is_some() || select.rename.is_some() {
        return None;
    }
    let Some(Source::Table(table)) = select.source.clone() else {
        return None;
    };
    select.limit = Some(select.limit.map_or(2, |limit| limit.min(2)));
    let path = crate::database::select_source(database, &table).ok()?;
    select.source = Some(Source::Table("part".into()));
    let mut output = BoundedOutput::default();
    if rusthouse_query::execute(&select, Some(&path), &mut output).is_err() {
        return None;
    }
    decode_single_value(&output.bytes)
}

fn replace_one(sql: &str, open: usize, close: usize, literal: &str) -> String {
    let mut result = String::with_capacity(sql.len() + literal.len());
    result.push_str(&sql[..open]);
    result.push_str(literal);
    result.push_str(&sql[close + 1..]);
    result
}

fn find_replacement(sql: &str, database: &Path) -> Option<(usize, usize, String)> {
    let bytes = sql.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            b'-' | b'/' if skip_comment(sql, cursor) != cursor => {
                cursor = skip_comment(sql, cursor)
            }
            b'(' => {
                let Some(close) = matching_close(sql, cursor) else {
                    return None;
                };
                let body = sql[cursor + 1..close].trim();
                if body
                    .get(..6)
                    .is_some_and(|head| head.eq_ignore_ascii_case("SELECT"))
                    && !source_subquery(sql, cursor)
                {
                    if let Some(value) = evaluate(body, database) {
                        return Some((cursor, close, sql_literal(value.as_deref())));
                    }
                }
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    None
}

fn lift_factor_into_correlated_avg(sql: &str) -> Option<String> {
    let bytes = sql.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            b'-' | b'/' if skip_comment(sql, cursor) != cursor => {
                cursor = skip_comment(sql, cursor)
            }
            b'(' => {
                let close = matching_close(sql, cursor)?;
                let body = sql[cursor + 1..close].trim();
                let Some(select_end) = body
                    .get(..6)
                    .filter(|head| head.eq_ignore_ascii_case("SELECT"))
                    .map(str::len)
                else {
                    cursor += 1;
                    continue;
                };
                let projection = body[select_end..].trim_start();
                if !keyword_at(projection, 0, "AVG") || !has_qualified_column(body) {
                    cursor += 1;
                    continue;
                }

                let mut star = cursor;
                while star > 0 && bytes[star - 1].is_ascii_whitespace() {
                    star -= 1;
                }
                if star == 0 || bytes[star - 1] != b'*' {
                    cursor += 1;
                    continue;
                }
                star -= 1;
                let mut number_end = star;
                while number_end > 0 && bytes[number_end - 1].is_ascii_whitespace() {
                    number_end -= 1;
                }
                let mut number_start = number_end;
                while number_start > 0
                    && (bytes[number_start - 1].is_ascii_digit() || bytes[number_start - 1] == b'.')
                {
                    number_start -= 1;
                }
                if number_start == number_end {
                    cursor += 1;
                    continue;
                }
                let factor = &sql[number_start..number_end];
                if !factor.parse::<f64>().is_ok_and(f64::is_finite)
                    || (number_start > 0
                        && (identifier_byte(bytes[number_start - 1])
                            || bytes[number_start - 1] == b'.'))
                {
                    cursor += 1;
                    continue;
                }
                let mut rewritten = String::with_capacity(sql.len());
                rewritten.push_str(&sql[..number_start]);
                rewritten.push_str("(SELECT ");
                rewritten.push_str(factor.trim());
                rewritten.push_str(" * ");
                rewritten.push_str(projection);
                rewritten.push(')');
                rewritten.push_str(&sql[close + 1..]);
                return Some(rewritten);
            }
            _ => cursor += 1,
        }
    }
    None
}

pub(super) fn has_candidate(sql: &str) -> bool {
    if correlated_count_span(sql).is_some() {
        return true;
    }
    let bytes = sql.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            b'-' | b'/' if skip_comment(sql, cursor) != cursor => {
                cursor = skip_comment(sql, cursor)
            }
            b'(' => {
                let Some(close) = matching_close(sql, cursor) else {
                    return false;
                };
                let body = sql[cursor + 1..close].trim();
                if body
                    .get(..6)
                    .is_some_and(|head| head.eq_ignore_ascii_case("SELECT"))
                    && !source_subquery(sql, cursor)
                    && !has_qualified_column(body)
                {
                    return true;
                }
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    false
}

pub(super) fn rewrite(query: &str, database: &Path) -> Result<Option<String>, String> {
    if !query.contains("(") || !query.to_ascii_uppercase().contains("SELECT") {
        return Ok(None);
    }
    if let Some(rewritten) = rewrite_correlated_count_positive(query, database) {
        return Ok(Some(normalize_distinct_parentheses(&rewritten)));
    }
    let mut rewritten = query.to_owned();
    let mut replacements = 0usize;
    while let Some((open, close, literal)) = find_replacement(&rewritten, database) {
        if replacements == MAX_SCALAR_SUBQUERIES {
            return Err("query contains more than 64 uncorrelated scalar subqueries".into());
        }
        rewritten = replace_one(&rewritten, open, close, &literal);
        replacements += 1;
    }
    while let Some(normalized) = lift_factor_into_correlated_avg(&rewritten) {
        rewritten = normalized;
    }
    if let Some(folded) = fold_constant_projection(&rewritten) {
        rewritten = folded;
    }
    Ok((replacements > 0).then_some(rewritten))
}

fn strip_alias(item: &str) -> (&str, Option<&str>) {
    let bytes = item.as_bytes();
    let mut cursor = 0usize;
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
        if !alias.is_empty()
            && alias.bytes().all(|byte| identifier_byte(byte))
            && alias
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$'))
        {
            return (item[..index].trim(), Some(alias));
        }
    }
    (item.trim(), None)
}

fn split_projection(sql: &str, start: usize, end: usize) -> Vec<&str> {
    let bytes = sql.as_bytes();
    let mut values = Vec::new();
    let mut depth = 0usize;
    let mut begin = start;
    let mut cursor = start;
    while cursor < end {
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
            b',' if depth == 0 => {
                values.push(sql[begin..cursor].trim());
                begin = cursor + 1;
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    values.push(sql[begin..end].trim());
    values
}

fn literal_expression(expression: &str) -> Option<String> {
    let expression = parse_scalar_expression(expression).ok()?;
    constant_value(&expression).map(|value| sql_literal(value.as_deref()))
}

fn constant_value(expression: &ScalarExpr) -> Option<Option<String>> {
    use ScalarExpr as Expr;
    let value = match expression {
        Expr::Integer(value) => Some(value.to_string()),
        Expr::Decimal(value) => Some(value.clone()),
        Expr::Column(_)
        | Expr::TextLength(_)
        | Expr::TextLiteral(_)
        | Expr::TextConcat(_)
        | Expr::TextSubstring { .. }
        | Expr::IsNull { .. }
        | Expr::TextCompare { .. }
        | Expr::TextLike { .. } => {
            return None;
        }
        Expr::Add(left, right)
        | Expr::Subtract(left, right)
        | Expr::Multiply(left, right)
        | Expr::Divide(left, right)
        | Expr::Modulo(left, right) => {
            let left = constant_value(left)?;
            let right = constant_value(right)?;
            let (Some(left), Some(right)) = (left, right) else {
                return Some(None);
            };
            let (Ok(left), Ok(right)) = (left.parse::<f64>(), right.parse::<f64>()) else {
                return None;
            };
            let result = match expression {
                Expr::Add(..) => left + right,
                Expr::Subtract(..) => left - right,
                Expr::Multiply(..) => left * right,
                Expr::Divide(..) => left / right,
                Expr::Modulo(..) => left % right,
                _ => unreachable!(),
            };
            if !result.is_finite() {
                return None;
            }
            Some(result.to_string())
        }
        Expr::Compare(left, operator, right) => {
            let left = constant_value(left)?;
            let right = constant_value(right)?;
            let (Some(left), Some(right)) = (left, right) else {
                return Some(None);
            };
            let (Ok(left), Ok(right)) = (left.parse::<f64>(), right.parse::<f64>()) else {
                return None;
            };
            use rusthouse_query::Operator;
            let result = match operator {
                Operator::Eq => left == right,
                Operator::Ne => left != right,
                Operator::Lt => left < right,
                Operator::Le => left <= right,
                Operator::Gt => left > right,
                Operator::Ge => left >= right,
                Operator::Modulo(_) | Operator::ModuloNe(_) => return None,
            };
            Some(if result { "TRUE" } else { "FALSE" }.into())
        }
        Expr::And(left, right) | Expr::Or(left, right) => {
            let left = constant_value(left)?;
            let right = constant_value(right)?;
            let (Some(left), Some(right)) = (left, right) else {
                return Some(None);
            };
            let truth = |value: &str| {
                value.eq_ignore_ascii_case("TRUE") || value.parse::<f64>().is_ok_and(|n| n != 0.0)
            };
            let result = if matches!(expression, Expr::And(..)) {
                truth(&left) && truth(&right)
            } else {
                truth(&left) || truth(&right)
            };
            Some(if result { "TRUE" } else { "FALSE" }.into())
        }
        Expr::Case {
            branches,
            else_expr,
        } => {
            for (condition, result) in branches {
                let Some(condition) = constant_value(condition)? else {
                    return Some(None);
                };
                let matched = condition.eq_ignore_ascii_case("TRUE")
                    || condition.parse::<f64>().is_ok_and(|value| value != 0.0);
                if matched {
                    return constant_value(result);
                }
            }
            if let Some(otherwise) = else_expr {
                constant_value(otherwise)?
            } else {
                None
            }
        }
    };
    Some(value)
}

/// Fold constant expressions in a SELECT list after scalar subqueries were
/// substituted. This is useful for searched CASE outputs composed only of
/// scalar aggregate results.
pub(super) fn fold_constant_projection(query: &str) -> Option<String> {
    let select_start = leading_select_start(query)?;
    let projection_start = select_start + 6;
    let from = find_top_level_from(query, projection_start)?;
    let mut items = Vec::new();
    let mut changed = false;
    for item in split_projection(query, projection_start, from) {
        let (expression, alias) = strip_alias(item);
        if let Some(literal) = literal_expression(expression) {
            items.push(match alias {
                Some(alias) => format!("{literal} AS {alias}"),
                None => literal,
            });
            changed = true;
        } else {
            items.push(item.to_owned());
        }
    }
    changed.then(|| {
        let mut rewritten = query[..projection_start].to_owned();
        if !rewritten
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_whitespace)
        {
            rewritten.push(' ');
        }
        rewritten.push_str(&items.join(", "));
        rewritten.push(' ');
        rewritten.push_str(&query[from..]);
        rewritten
    })
}

fn find_top_level_from(sql: &str, start: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut cursor = start;
    let mut depth = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\'' | b'"' | b'`' => cursor = skip_quoted(sql, cursor, bytes[cursor]),
            b'-' | b'/' if skip_comment(sql, cursor) != cursor => {
                cursor = skip_comment(sql, cursor)
            }
            b'(' => {
                depth += 1;
                cursor += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                cursor += 1;
            }
            _ if depth == 0 && keyword_at(sql, cursor, "FROM") => return Some(cursor),
            _ => cursor += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_correlated_and_exists_subqueries_intact() {
        let query = "SELECT x FROM t WHERE x > (SELECT AVG(y) FROM u WHERE u.k = t.k) AND EXISTS (SELECT 1 FROM z)";
        assert!(find_replacement(query, Path::new("unused")).is_none());
    }

    #[test]
    fn keeps_derived_sources_intact() {
        let query = "SELECT x FROM (SELECT x FROM t) d, (SELECT y FROM u) e";
        assert!(find_replacement(query, Path::new("unused")).is_none());
    }

    #[test]
    fn identifies_only_uncorrelated_scalar_candidates() {
        assert!(has_candidate(
            "SELECT x FROM t WHERE x = (SELECT MAX(y) FROM u)"
        ));
        assert!(!has_candidate(
            "SELECT x FROM t WHERE x = (SELECT MAX(y) FROM u WHERE u.k = t.k)"
        ));
        assert!(!has_candidate(
            "SELECT x FROM t WHERE x IN (SELECT y FROM u)"
        ));
    }

    #[test]
    fn recognizes_correlated_count_positive_with_a_shared_outer_key() {
        let query = "SELECT DISTINCT (name) FROM items i WHERE id BETWEEN 1 AND 4 AND (SELECT COUNT(*) AS n FROM items WHERE (key = i.key AND flag = 'a') OR (key = i.key AND flag = 'b')) > 0 ORDER BY name";
        let span = correlated_count_span(query).expect("bounded correlated COUNT(*)");
        let candidate_query = correlated_count_candidate_query(query, &span)
            .expect("outer table and candidate bounds");

        assert_eq!(span.key, "key");
        assert!(query[span.open..span.end].contains("COUNT(*)"));
        assert!(candidate_query.contains("AND key IS NOT NULL"));
        assert!(parse_select(&candidate_query).is_ok(), "{candidate_query}");
        assert!(has_candidate(query));
        assert_eq!(
            normalize_distinct_parentheses(query),
            "SELECT DISTINCT name FROM items i WHERE id BETWEEN 1 AND 4 AND (SELECT COUNT(*) AS n FROM items WHERE (key = i.key AND flag = 'a') OR (key = i.key AND flag = 'b')) > 0 ORDER BY name"
        );
    }

    #[test]
    fn empty_correlated_count_uses_a_parseable_always_false_filter() {
        let filter = correlated_count_filter("key", &[]);

        assert_eq!(filter, "key IS NULL AND key IS NOT NULL");
        assert!(parse_select(&format!("SELECT key FROM items WHERE {filter}")).is_ok());
    }

    #[test]
    fn decimal_literals_are_not_mistaken_for_column_qualification() {
        assert!(!has_qualified_column(
            "SELECT AVG(amount) FROM sales WHERE amount > 1.25"
        ));
        assert!(has_qualified_column(
            "SELECT AVG(u.amount) FROM sales u WHERE u.id = t.id"
        ));
    }

    #[test]
    fn ignores_subquery_text_inside_literals_and_comments() {
        let query = "SELECT '(SELECT 1)' AS text FROM t -- (SELECT 2)\n";
        assert!(find_replacement(query, Path::new("unused")).is_none());
    }

    #[test]
    fn recognizes_select_after_leading_comments() {
        assert!(starts_with_select("-- TPC-DS benchmark\nSELECT 1"));
        assert!(starts_with_select(" /* note */ SELECT 1"));
        assert!(!starts_with_select("-- only a comment"));
    }

    #[test]
    fn folds_scalar_case_expressions_after_substitution() {
        let query = "SELECT CASE WHEN 2 > 1 THEN 4.5 ELSE 0 END result, x FROM t";
        assert_eq!(
            fold_constant_projection(query).as_deref(),
            Some("SELECT 4.5 AS result, x FROM t")
        );
    }

    #[test]
    fn folds_projection_after_a_benchmark_comment() {
        assert_eq!(
            fold_constant_projection(
                "-- TPC-DS Query 9\nSELECT CASE WHEN 2 > 1 THEN 4.5 ELSE 0 END result FROM reason"
            )
            .as_deref(),
            Some("-- TPC-DS Query 9\nSELECT 4.5 AS result FROM reason")
        );
    }

    #[test]
    fn removes_parentheses_from_tpcds_distinct_scalar_projection() {
        assert_eq!(
            normalize_distinct_parentheses(
                "SELECT DISTINCT (d_month_seq) FROM date_dim WHERE d_year = 2001"
            ),
            "SELECT DISTINCT d_month_seq FROM date_dim WHERE d_year = 2001"
        );
        assert_eq!(
            normalize_distinct_parentheses("SELECT distinct(d_month_seq) FROM date_dim"),
            "SELECT distinct d_month_seq FROM date_dim"
        );
    }

    #[test]
    fn moves_a_correlated_avg_factor_into_its_scalar_select() {
        assert_eq!(
            lift_factor_into_correlated_avg(
                "SELECT x FROM t WHERE x > 1.2 * (SELECT AVG(u.value) FROM u WHERE u.k = t.k)"
            )
            .as_deref(),
            Some("SELECT x FROM t WHERE x > (SELECT 1.2 * AVG(u.value) FROM u WHERE u.k = t.k)")
        );
    }

    #[test]
    fn renders_scalar_values_as_safe_sql_literals() {
        assert_eq!(sql_literal(Some("1.25")), "1.25");
        assert_eq!(sql_literal(Some("true")), "TRUE");
        assert_eq!(sql_literal(Some("O'Brien")), "'O''Brien'");
        assert_eq!(sql_literal(None), "NULL");
    }

    #[test]
    fn leaves_column_dependent_case_expressions_unchanged() {
        let query = "SELECT CASE WHEN x > 1 THEN 4 ELSE 0 END result FROM t";
        assert!(fold_constant_projection(query).is_none());
    }
}
