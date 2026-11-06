//! Execute a grouped derived table as a bounded temporary source for an outer join.
use super::{
    JoinColumn, JoinGroupedOutput, JoinSelect, JoinType, ParseError, Token, parse_join_with_schema,
};
use crate::{Operator, tokenize};

const DERIVED_TABLE: &str = "__rusthouse_derived_input";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedColumn {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedGroupedJoin {
    pub(crate) inner: JoinSelect,
    pub(crate) outer: JoinSelect,
    pub(crate) table: String,
    pub(crate) columns: Vec<DerivedColumn>,
}

impl DerivedGroupedJoin {
    pub fn inner(&self) -> &JoinSelect {
        &self.inner
    }

    pub fn outer(&self) -> &JoinSelect {
        &self.outer
    }

    pub fn table(&self) -> &str {
        &self.table
    }

    pub fn columns(&self) -> &[DerivedColumn] {
        &self.columns
    }

    pub fn cap_rows(&mut self, cap: usize) {
        self.outer.limit = Some(self.outer.limit.map_or(cap, |limit| limit.min(cap)));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DerivedGroupedOrderKey {
    Column(usize),
    Difference { left: usize, right: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DerivedGroupedOrder {
    pub key: DerivedGroupedOrderKey,
    pub descending: bool,
    pub nulls_first: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DerivedGroupedFilter {
    WindowDeviation {
        average_index: usize,
        sum_index: usize,
        threshold: String,
    },
    RatioBetween {
        numerator_index: usize,
        denominator_index: usize,
        scale: String,
        lower: String,
        upper: String,
    },
}

/// A grouped join wrapped by the TPC-DS window-average deviation filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedGroupedSelect {
    pub(crate) inner: JoinSelect,
    pub(crate) columns: Vec<DerivedColumn>,
    pub(crate) filter: DerivedGroupedFilter,
    pub(crate) order: Vec<DerivedGroupedOrder>,
    pub(crate) limit: Option<usize>,
    pub(crate) offset: usize,
}

impl DerivedGroupedSelect {
    pub fn inner(&self) -> &JoinSelect {
        &self.inner
    }

    pub fn columns(&self) -> &[DerivedColumn] {
        &self.columns
    }

    pub fn cap_rows(&mut self, cap: usize) {
        self.limit = Some(self.limit.map_or(cap, |limit| limit.min(cap)));
    }
}

fn keyword(token: &Token, name: &str) -> bool {
    matches!(token, Token::Word(word) if word.eq_ignore_ascii_case(name))
}

fn render_token(token: &Token) -> String {
    match token {
        Token::Word(value) | Token::Number(value) => value.clone(),
        Token::QuotedWord(value) | Token::QuotedAlias(value) => {
            format!("\"{}\"", value.replace('"', "\"\""))
        }
        Token::String(value) => format!("'{}'", value.replace('\'', "''")),
        Token::Star => "*".into(),
        Token::Slash => "/".into(),
        Token::Percent => "%".into(),
        Token::Plus => "+".into(),
        Token::Minus => "-".into(),
        Token::Comma => ",".into(),
        Token::Dot => ".".into(),
        Token::LParen => "(".into(),
        Token::RParen => ")".into(),
        Token::Semicolon => ";".into(),
        Token::Operator(operator) => match operator {
            Operator::Eq => "=".into(),
            Operator::Ne => "!=".into(),
            Operator::Lt => "<".into(),
            Operator::Le => "<=".into(),
            Operator::Gt => ">".into(),
            Operator::Ge => ">=".into(),
            Operator::Modulo(value) => format!("%{value}"),
            Operator::ModuloNe(value) => format!("%{value} !="),
        },
    }
}

fn render_tokens(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(render_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_simple_bound(token: &Token) -> bool {
    matches!(token, Token::Number(_) | Token::String(_))
}

/// The basic JOIN predicate parser accepts comparisons joined by AND. Expand
/// a simple BETWEEN into that form for the outer query over the materialized
/// source, preserving SQL's predicate precedence.
fn expand_simple_between(tokens: &[Token]) -> Vec<Token> {
    let mut expanded = Vec::with_capacity(tokens.len());
    let mut index = 0;
    while index < tokens.len() {
        if keyword(&tokens[index], "BETWEEN") && index >= 2 {
            let (lhs_start, lhs_end) =
                if matches!(tokens[index - 1], Token::Word(_) | Token::QuotedWord(_)) {
                    if index >= 4 && tokens[index - 2] == Token::Dot {
                        (index - 3, index)
                    } else {
                        (index - 1, index)
                    }
                } else {
                    (index, index)
                };
            if lhs_start < lhs_end
                && is_simple_bound(&tokens[index + 1])
                && tokens
                    .get(index + 2)
                    .is_some_and(|token| keyword(token, "AND"))
                && tokens.get(index + 3).is_some_and(is_simple_bound)
            {
                let lhs = &tokens[lhs_start..lhs_end];
                expanded.truncate(expanded.len() - lhs.len());
                expanded.extend_from_slice(lhs);
                expanded.push(Token::Operator(Operator::Ge));
                expanded.push(tokens[index + 1].clone());
                expanded.push(Token::Word("AND".into()));
                expanded.extend_from_slice(lhs);
                expanded.push(Token::Operator(Operator::Le));
                expanded.push(tokens[index + 3].clone());
                index += 4;
                continue;
            }
        }
        expanded.push(tokens[index].clone());
        index += 1;
    }
    expanded
}

fn derived_bounds(tokens: &[Token]) -> Option<(usize, usize)> {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ if depth == 0 && keyword(token, "FROM") => {
                if tokens.get(index + 1) != Some(&Token::LParen)
                    || !tokens
                        .get(index + 2)
                        .is_some_and(|token| keyword(token, "SELECT"))
                {
                    return None;
                }
                let mut end = index + 2;
                depth = 1;
                while end < tokens.len() && depth > 0 {
                    match tokens[end] {
                        Token::LParen => depth += 1,
                        Token::RParen => depth -= 1,
                        _ => {}
                    }
                    end += 1;
                }
                return (depth == 0).then_some((index, end - 1));
            }
            _ => {}
        }
    }
    None
}

fn union_derived_bounds(tokens: &[Token]) -> Option<(usize, usize)> {
    let mut depth = 0usize;
    let mut in_from = false;
    for (index, token) in tokens.iter().enumerate() {
        if depth == 0 && keyword(token, "FROM") {
            in_from = true;
            continue;
        }
        if depth == 0
            && in_from
            && matches!(token, Token::Word(word) if ["WHERE", "GROUP", "ORDER", "LIMIT", "OFFSET", "HAVING"].iter().any(|name| word.eq_ignore_ascii_case(name)))
        {
            break;
        }
        if depth == 0
            && in_from
            && *token == Token::LParen
            && tokens
                .get(index + 1)
                .is_some_and(|token| keyword(token, "SELECT"))
        {
            let mut close = index + 1;
            depth = 1;
            while close < tokens.len() && depth > 0 {
                match tokens[close] {
                    Token::LParen => depth += 1,
                    Token::RParen => depth -= 1,
                    _ => {}
                }
                close += 1;
            }
            return (depth == 0).then_some((index, close - 1));
        }
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

fn split_union_all(tokens: &[Token]) -> Option<Vec<&[Token]>> {
    let mut branches = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut index = 0usize;
    while index < tokens.len() {
        if depth == 0 && keyword(&tokens[index], "UNION") {
            if !tokens
                .get(index + 1)
                .is_some_and(|token| keyword(token, "ALL"))
                || start == index
            {
                return None;
            }
            branches.push(&tokens[start..index]);
            index += 2;
            start = index;
            continue;
        }
        match tokens[index] {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
        index += 1;
    }
    if branches.is_empty() || start == tokens.len() {
        return None;
    }
    branches.push(&tokens[start..]);
    (branches.len() <= 8).then_some(branches)
}

fn sources(query: &JoinSelect) -> Vec<(&str, &str)> {
    let mut sources = vec![
        (query.left.as_str(), query.left_alias.as_str()),
        (query.right.as_str(), query.right_alias.as_str()),
    ];
    sources.extend(
        query
            .steps
            .iter()
            .map(|step| (step.table.as_str(), step.alias.as_str())),
    );
    sources
}

fn source_type(
    query: &JoinSelect,
    column: &JoinColumn,
    resolve: &mut impl FnMut(&str) -> Result<Vec<DerivedColumn>, ParseError>,
) -> Result<Option<String>, ParseError> {
    let mut found = None;
    for (table, source_alias) in sources(query) {
        if !column.relation.is_empty()
            && column.relation != source_alias
            && column.relation != table
        {
            continue;
        }
        let schema = resolve(table)?;
        if let Some(metadata) = schema.iter().find(|item| item.name == column.name) {
            if found.replace(metadata.data_type.clone()).is_some() {
                return Ok(None);
            }
        }
    }
    Ok(found)
}

fn grouped_select_columns(
    inner: &JoinSelect,
    resolve: &mut impl FnMut(&str) -> Result<Vec<DerivedColumn>, ParseError>,
) -> Result<Option<Vec<DerivedColumn>>, ParseError> {
    if inner.kind != JoinType::Inner
        || inner.steps.iter().any(|step| step.kind != JoinType::Inner)
        || inner.grouped_rollup
        || inner.limit.is_some()
        || inner.offset != 0
        || !inner.order_by.is_empty()
        || inner.grouped_window_average.is_some()
        || inner.grouped_window_ratio.is_some()
    {
        return Ok(None);
    }
    let Some(keys) = inner.columns.as_ref() else {
        return Ok(None);
    };

    let mut columns = Vec::new();
    if inner.grouped_output.is_empty() {
        if inner.grouped_case_counts.is_empty() {
            return Ok(None);
        }
        for (index, key) in keys.iter().enumerate() {
            let Some(data_type) = source_type(inner, key, resolve)? else {
                return Ok(None);
            };
            let name = inner
                .grouped_key_aliases
                .iter()
                .find_map(|(position, alias)| (*position == index).then(|| alias.clone()))
                .unwrap_or_else(|| key.name.clone());
            columns.push(DerivedColumn { name, data_type });
        }
        columns.extend(
            inner
                .grouped_case_counts
                .iter()
                .map(|(_, _, name)| DerivedColumn {
                    name: name.clone(),
                    data_type: "Nullable(Float64)".into(),
                }),
        );
    } else {
        for item in &inner.grouped_output {
            let column = match item {
                JoinGroupedOutput::Key(index) => {
                    let Some(key) = keys.get(*index) else {
                        return Ok(None);
                    };
                    let Some(data_type) = source_type(inner, key, resolve)? else {
                        return Ok(None);
                    };
                    let name = inner
                        .grouped_key_aliases
                        .iter()
                        .find_map(|(position, alias)| (*position == *index).then(|| alias.clone()))
                        .unwrap_or_else(|| key.name.clone());
                    DerivedColumn { name, data_type }
                }
                JoinGroupedOutput::Count(name) => DerivedColumn {
                    name: name.clone(),
                    data_type: "UInt64".into(),
                },
                JoinGroupedOutput::Sum(name)
                | JoinGroupedOutput::SumExtra(_, name)
                | JoinGroupedOutput::WindowRatio(name)
                | JoinGroupedOutput::WindowAverage(name) => DerivedColumn {
                    name: name.clone(),
                    data_type: "Nullable(Float64)".into(),
                },
                JoinGroupedOutput::Min(_, _)
                | JoinGroupedOutput::Max(_, _)
                | JoinGroupedOutput::Average(_, _) => return Ok(None),
            };
            columns.push(column);
        }
    }

    if columns.is_empty()
        || columns.iter().enumerate().any(|(index, column)| {
            columns[..index]
                .iter()
                .any(|previous| previous.name.eq_ignore_ascii_case(&column.name))
        })
    {
        return Ok(None);
    }
    Ok(Some(columns))
}

fn grouped_count_columns(
    inner: &JoinSelect,
    resolve: &mut impl FnMut(&str) -> Result<Vec<DerivedColumn>, ParseError>,
) -> Result<Option<Vec<DerivedColumn>>, ParseError> {
    let (Some(keys), Some(grouped_keys)) = (&inner.columns, &inner.grouped_count) else {
        return Ok(None);
    };
    if keys.len() != grouped_keys.len()
        || inner.grouped_keys_only
        || inner.grouped_count_column.is_some()
        || inner.grouped_distinct.is_some()
        || !inner.grouped_sum_extras.is_empty()
        || inner.grouped_min.is_some()
        || !inner.grouped_min_metrics.is_empty()
        || !inner.grouped_max_metrics.is_empty()
        || !inner.grouped_avg_metrics.is_empty()
        || !inner.grouped_case_counts.is_empty()
        || inner.grouped_formula.is_some()
        || inner.grouped_window_ratio.is_some()
        || inner.grouped_year.is_some()
        || inner.limit.is_some()
        || inner.offset != 0
        || !inner.order_by.is_empty()
        || inner.kind != super::JoinType::Inner
        || inner
            .steps
            .iter()
            .any(|step| step.kind != super::JoinType::Inner)
    {
        return Ok(None);
    }
    let Some(sum_column) = inner.grouped_sum.as_ref().and_then(|sum| match sum {
        super::GroupedSum::Column(column) => Some(column),
        super::GroupedSum::Expression(_) => None,
    }) else {
        if inner.grouped_sum.is_some() {
            return Ok(None);
        }
        let metric_name = inner
            .aggregate_alias
            .clone()
            .unwrap_or_else(|| "count".into());
        if keys
            .iter()
            .any(|key| key.name.eq_ignore_ascii_case(&metric_name))
        {
            return Ok(None);
        }
        let metric_position = inner.grouped_sum_position.unwrap_or(keys.len());
        if metric_position > keys.len() {
            return Ok(None);
        }
        let mut columns = Vec::with_capacity(keys.len() + 1);
        let mut key_index = 0usize;
        for position in 0..=keys.len() {
            if position == metric_position {
                columns.push(DerivedColumn {
                    name: metric_name.clone(),
                    data_type: "UInt64".into(),
                });
                continue;
            }
            let key = &keys[key_index];
            let Some(data_type) = source_type(inner, key, resolve)? else {
                return Ok(None);
            };
            let name = inner
                .grouped_key_aliases
                .iter()
                .find_map(|(index, alias)| (*index == key_index).then(|| alias.clone()))
                .unwrap_or_else(|| key.name.clone());
            columns.push(DerivedColumn { name, data_type });
            key_index += 1;
        }
        return Ok(Some(columns));
    };

    if let Some(window_average) = &inner.grouped_window_average {
        if inner.grouped_execution_keys.is_empty()
            || window_average
                .partition
                .iter()
                .any(|key| !inner.grouped_execution_keys.contains(key))
        {
            return Ok(None);
        }
    }
    let Some(sum_type) = source_type(inner, sum_column, resolve)? else {
        return Ok(None);
    };
    let sum_type = if sum_type.starts_with("Nullable(") {
        sum_type
    } else {
        format!("Nullable({sum_type})")
    };
    let mut columns = Vec::with_capacity(inner.grouped_output.len());
    for item in &inner.grouped_output {
        let column = match item {
            super::JoinGroupedOutput::Key(index) => {
                let Some(key) = keys.get(*index) else {
                    return Ok(None);
                };
                let Some(data_type) = source_type(inner, key, resolve)? else {
                    return Ok(None);
                };
                let name = inner
                    .grouped_key_aliases
                    .iter()
                    .find_map(|(position, alias)| (*position == *index).then(|| alias.clone()))
                    .unwrap_or_else(|| key.name.clone());
                DerivedColumn { name, data_type }
            }
            super::JoinGroupedOutput::Count(name) => DerivedColumn {
                name: name.clone(),
                data_type: "UInt64".into(),
            },
            super::JoinGroupedOutput::Sum(name) => DerivedColumn {
                name: name.clone(),
                data_type: sum_type.clone(),
            },
            super::JoinGroupedOutput::WindowAverage(name) => DerivedColumn {
                name: name.clone(),
                data_type: "Nullable(Float64)".into(),
            },
            _ => return Ok(None),
        };
        if columns
            .iter()
            .any(|previous: &DerivedColumn| previous.name.eq_ignore_ascii_case(&column.name))
        {
            return Ok(None);
        }
        columns.push(column);
    }
    Ok(Some(columns))
}

fn token_name(token: &Token) -> Option<&str> {
    match token {
        Token::Word(name) | Token::QuotedWord(name) | Token::QuotedAlias(name) => Some(name),
        _ => None,
    }
}

fn find_top_level_clause(tokens: &[Token], start: usize, name: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(start) {
        if depth == 0 && keyword(token, name) {
            return Some(index);
        }
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

fn grouped_window_filter_threshold(tokens: &[Token], average: &str, sum: &str) -> Option<String> {
    let normalized = tokens
        .iter()
        .filter(|token| !matches!(token, Token::LParen | Token::RParen))
        .collect::<Vec<_>>();
    if normalized.len() != 17
        || !keyword(normalized[0], "CASE")
        || !keyword(normalized[1], "WHEN")
        || !token_name(normalized[2]).is_some_and(|name| name.eq_ignore_ascii_case(average))
        || !matches!(normalized[3], Token::Operator(Operator::Gt | Operator::Ne))
        || !matches!(normalized[4], Token::Number(value) if value == "0")
        || !keyword(normalized[5], "THEN")
        || !keyword(normalized[6], "ABS")
        || !token_name(normalized[7]).is_some_and(|name| name.eq_ignore_ascii_case(sum))
        || normalized[8] != &Token::Minus
        || !token_name(normalized[9]).is_some_and(|name| name.eq_ignore_ascii_case(average))
        || normalized[10] != &Token::Slash
        || !token_name(normalized[11]).is_some_and(|name| name.eq_ignore_ascii_case(average))
        || !keyword(normalized[12], "ELSE")
        || !keyword(normalized[13], "NULL")
        || !keyword(normalized[14], "END")
        || normalized[15] != &Token::Operator(Operator::Gt)
    {
        return None;
    }
    let Token::Number(threshold) = normalized[16] else {
        return None;
    };
    threshold
        .parse::<f64>()
        .is_ok_and(|value| value.is_finite() && value > 0.0)
        .then(|| threshold.clone())
}

fn split_order_terms(tokens: &[Token]) -> Vec<&[Token]> {
    let mut terms = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Comma if depth == 0 => {
                terms.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if start < tokens.len() {
        terms.push(&tokens[start..]);
    }
    terms
}

fn parse_grouped_select_order(
    tokens: &[Token],
    columns: &[DerivedColumn],
) -> Option<Vec<DerivedGroupedOrder>> {
    split_order_terms(tokens)
        .into_iter()
        .map(|term| {
            let mut term = term;
            let nulls_first = if term.len() >= 3
                && term
                    .get(term.len() - 2)
                    .and_then(|token| token_name(token))
                    .is_some_and(|name| name.eq_ignore_ascii_case("NULLS"))
            {
                let first = term
                    .last()
                    .and_then(|token| token_name(token))
                    .filter(|name| {
                        name.eq_ignore_ascii_case("FIRST") || name.eq_ignore_ascii_case("LAST")
                    })?;
                term = &term[..term.len() - 2];
                Some(first.eq_ignore_ascii_case("FIRST"))
            } else {
                None
            };
            let descending = match term.last().and_then(|token| token_name(token)) {
                Some(name) if name.eq_ignore_ascii_case("DESC") => {
                    term = &term[..term.len() - 1];
                    true
                }
                Some(name) if name.eq_ignore_ascii_case("ASC") => {
                    term = &term[..term.len() - 1];
                    false
                }
                _ => false,
            };
            let key = match term {
                [Token::Number(position)] => {
                    let position = position.parse::<usize>().ok()?;
                    if position == 0 || position > columns.len() {
                        return None;
                    }
                    DerivedGroupedOrderKey::Column(position - 1)
                }
                [left, Token::Minus, right] => {
                    let left = token_name(left)?;
                    let right = token_name(right)?;
                    let left = columns
                        .iter()
                        .position(|column| column.name.eq_ignore_ascii_case(left))?;
                    let right = columns
                        .iter()
                        .position(|column| column.name.eq_ignore_ascii_case(right))?;
                    DerivedGroupedOrderKey::Difference { left, right }
                }
                [name] => {
                    let name = token_name(name)?;
                    let index = columns
                        .iter()
                        .position(|column| column.name.eq_ignore_ascii_case(name))?;
                    DerivedGroupedOrderKey::Column(index)
                }
                _ => return None,
            };
            Some(DerivedGroupedOrder {
                key,
                descending,
                nulls_first,
            })
        })
        .collect()
}

fn ratio_value(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/').unwrap_or((value, "1"));
    let numerator = numerator.parse::<f64>().ok()?;
    let denominator = denominator.parse::<f64>().ok()?;
    if !numerator.is_finite() || !denominator.is_finite() || denominator == 0.0 {
        return None;
    }
    let result = numerator / denominator;
    result.is_finite().then_some(result)
}

fn ratio_literal(tokens: &[&Token]) -> Option<String> {
    let value = match tokens {
        [Token::Number(value)] => value.clone(),
        [
            Token::Number(numerator),
            Token::Slash,
            Token::Number(denominator),
        ] => {
            format!("{numerator}/{denominator}")
        }
        _ => return None,
    };
    ratio_value(&value).map(|_| value)
}

fn parse_ratio_between_filter(
    tokens: &[Token],
    columns: &[DerivedColumn],
) -> Option<DerivedGroupedFilter> {
    let normalized = tokens
        .iter()
        .filter(|token| !matches!(token, Token::LParen | Token::RParen))
        .collect::<Vec<_>>();
    if !normalized
        .first()
        .is_some_and(|token| keyword(token, "CASE"))
        || !normalized
            .get(1)
            .is_some_and(|token| keyword(token, "WHEN"))
        || !normalized
            .get(3)
            .is_some_and(|token| **token == Token::Operator(Operator::Gt))
        || !matches!(normalized.get(4), Some(Token::Number(value)) if value == "0")
        || !normalized
            .get(5)
            .is_some_and(|token| keyword(token, "THEN"))
    {
        return None;
    }
    let denominator = token_name(normalized.get(2)?)?;
    let else_index = normalized.iter().position(|token| keyword(token, "ELSE"))?;
    let between_index = else_index + 3;
    if !normalized
        .get(else_index + 1)
        .is_some_and(|token| keyword(token, "NULL"))
        || !normalized
            .get(else_index + 2)
            .is_some_and(|token| keyword(token, "END"))
        || !normalized
            .get(between_index)
            .is_some_and(|token| keyword(token, "BETWEEN"))
    {
        return None;
    }
    let (numerator, scale, ratio_denominator) = match &normalized[6..else_index] {
        [numerator, Token::Slash, ratio_denominator] => (
            token_name(numerator)?,
            "1".to_owned(),
            token_name(ratio_denominator)?,
        ),
        [
            numerator,
            Token::Star,
            Token::Number(scale),
            Token::Slash,
            ratio_denominator,
        ] => (
            token_name(numerator)?,
            scale.clone(),
            token_name(ratio_denominator)?,
        ),
        _ => return None,
    };
    if !denominator.eq_ignore_ascii_case(ratio_denominator)
        || !ratio_value(&scale).is_some_and(|value| value.is_finite())
    {
        return None;
    }
    let and_index = normalized
        .iter()
        .enumerate()
        .skip(between_index + 1)
        .find_map(|(index, token)| keyword(token, "AND").then_some(index))?;
    let lower = ratio_literal(&normalized[between_index + 1..and_index])?;
    let upper = ratio_literal(&normalized[and_index + 1..])?;
    if ratio_value(&lower)? > ratio_value(&upper)? {
        return None;
    }
    let numerator_index = columns
        .iter()
        .position(|column| column.name.eq_ignore_ascii_case(numerator))?;
    let denominator_index = columns
        .iter()
        .position(|column| column.name.eq_ignore_ascii_case(denominator))?;
    Some(DerivedGroupedFilter::RatioBetween {
        numerator_index,
        denominator_index,
        scale,
        lower,
        upper,
    })
}

fn parse_derived_ratio_select(
    tokens: &[Token],
    resolve: &mut impl FnMut(&str) -> Result<Vec<DerivedColumn>, ParseError>,
) -> Result<Option<DerivedGroupedSelect>, ParseError> {
    if !tokens.first().is_some_and(|token| keyword(token, "SELECT"))
        || tokens.get(1) != Some(&Token::Star)
        || !tokens.get(2).is_some_and(|token| keyword(token, "FROM"))
    {
        return Ok(None);
    }
    let Some((from, close)) = derived_bounds(tokens) else {
        return Ok(None);
    };
    let inner_sql = render_tokens(&tokens[from + 2..close]);
    let Some(inner) = parse_join_with_schema(&inner_sql, |table| {
        resolve(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
    })?
    else {
        return Ok(None);
    };
    if inner.grouped_case_counts.is_empty() && inner.grouped_output.is_empty()
        || inner.grouped_window_average.is_some()
        || inner.grouped_window_ratio.is_some()
    {
        return Ok(None);
    }
    let Some(columns) = grouped_select_columns(&inner, resolve)? else {
        return Ok(None);
    };

    let mut where_index = close + 1;
    let has_as = tokens
        .get(where_index)
        .is_some_and(|token| keyword(token, "AS"));
    if has_as {
        where_index += 1;
    }
    if tokens
        .get(where_index)
        .is_some_and(|token| keyword(token, "WHERE"))
    {
        // A derived SELECT may omit its table alias.
        if has_as {
            return Ok(None);
        }
    } else if tokens
        .get(where_index)
        .is_some_and(|token| token_name(token).is_some())
    {
        where_index += 1;
    } else {
        return Ok(None);
    }
    if !tokens
        .get(where_index)
        .is_some_and(|token| keyword(token, "WHERE"))
    {
        return Ok(None);
    }
    let order_index = find_top_level_clause(tokens, where_index + 1, "ORDER");
    let limit_index = find_top_level_clause(tokens, where_index + 1, "LIMIT");
    let offset_index = find_top_level_clause(tokens, where_index + 1, "OFFSET");
    let clause_end = [order_index, limit_index, offset_index]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(tokens.len());
    let Some(filter) = parse_ratio_between_filter(&tokens[where_index + 1..clause_end], &columns)
    else {
        return Ok(None);
    };

    let order = if let Some(order_index) = order_index {
        if tokens
            .get(order_index + 1)
            .is_none_or(|token| !keyword(token, "BY"))
        {
            return Ok(None);
        }
        let order_end = [limit_index, offset_index]
            .into_iter()
            .flatten()
            .filter(|index| *index > order_index)
            .min()
            .unwrap_or(tokens.len());
        let Some(order) = parse_grouped_select_order(&tokens[order_index + 2..order_end], &columns)
        else {
            return Ok(None);
        };
        order
    } else {
        Vec::new()
    };
    let parse_bound = |index: Option<usize>| -> Result<Option<usize>, ParseError> {
        let Some(index) = index else {
            return Ok(None);
        };
        let Some(Token::Number(value)) = tokens.get(index + 1) else {
            return Err(ParseError(
                "derived grouped LIMIT/OFFSET requires a number".into(),
            ));
        };
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError(
                "derived grouped LIMIT/OFFSET must be non-negative".into(),
            ));
        }
        value
            .parse::<usize>()
            .map(Some)
            .map_err(|_| ParseError("derived grouped LIMIT/OFFSET is too large".into()))
    };
    let limit = parse_bound(limit_index)?;
    let offset = parse_bound(offset_index)?.unwrap_or(0);
    if tokens
        .iter()
        .enumerate()
        .any(|(index, token)| index > close && keyword(token, "FETCH"))
    {
        return Ok(None);
    }
    Ok(Some(DerivedGroupedSelect {
        inner,
        columns,
        filter,
        order,
        limit,
        offset,
    }))
}

/// Parse the bounded TPC-DS window-average deviation SELECTs (Q53/Q63/Q89).
pub fn parse_derived_grouped_select_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<DerivedColumn>, ParseError>,
) -> Result<Option<DerivedGroupedSelect>, ParseError> {
    let tokens = tokenize(sql)?;
    let has_window_average = tokens.windows(3).any(|items| {
        keyword(&items[0], "AVG") && items[1] == Token::LParen && keyword(&items[2], "SUM")
    });
    if !has_window_average {
        return parse_derived_ratio_select(&tokens, &mut resolve);
    }
    if !tokens.first().is_some_and(|token| keyword(token, "SELECT"))
        || tokens.get(1) != Some(&Token::Star)
    {
        return Ok(None);
    }
    let Some((from, close)) = derived_bounds(&tokens) else {
        return Ok(None);
    };
    let inner_sql = render_tokens(&tokens[from + 2..close]);
    let Some(inner) = parse_join_with_schema(&inner_sql, |table| {
        resolve(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
    })?
    else {
        return Ok(None);
    };
    let Some(window_average) = inner.grouped_window_average.as_ref() else {
        return Ok(None);
    };
    let Some(columns) = grouped_count_columns(&inner, &mut resolve)? else {
        return Ok(None);
    };
    let Some(sum_alias) = inner.grouped_output.iter().find_map(|item| match item {
        super::JoinGroupedOutput::Sum(alias) => Some(alias.as_str()),
        _ => None,
    }) else {
        return Ok(None);
    };
    let Some(average_index) = columns
        .iter()
        .position(|column| column.name.eq_ignore_ascii_case(&window_average.alias))
    else {
        return Ok(None);
    };
    let Some(sum_index) = columns
        .iter()
        .position(|column| column.name.eq_ignore_ascii_case(sum_alias))
    else {
        return Ok(None);
    };

    let where_index = find_top_level_clause(&tokens, close + 1, "WHERE");
    let Some(where_index) = where_index else {
        return Ok(None);
    };
    let order_index = find_top_level_clause(&tokens, where_index + 1, "ORDER");
    let limit_index = find_top_level_clause(&tokens, where_index + 1, "LIMIT");
    let offset_index = find_top_level_clause(&tokens, where_index + 1, "OFFSET");
    let clause_end = [order_index, limit_index, offset_index]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(tokens.len());
    let Some(threshold) = grouped_window_filter_threshold(
        &tokens[where_index + 1..clause_end],
        &window_average.alias,
        sum_alias,
    ) else {
        return Ok(None);
    };

    let order = if let Some(order_index) = order_index {
        if tokens
            .get(order_index + 1)
            .is_none_or(|token| !keyword(token, "BY"))
        {
            return Ok(None);
        }
        let order_end = [limit_index, offset_index]
            .into_iter()
            .flatten()
            .filter(|index| *index > order_index)
            .min()
            .unwrap_or(tokens.len());
        let Some(order) = parse_grouped_select_order(&tokens[order_index + 2..order_end], &columns)
        else {
            return Ok(None);
        };
        order
    } else {
        Vec::new()
    };
    let parse_bound = |index: Option<usize>| -> Result<Option<usize>, ParseError> {
        let Some(index) = index else {
            return Ok(None);
        };
        let Some(Token::Number(value)) = tokens.get(index + 1) else {
            return Err(ParseError(
                "derived grouped LIMIT/OFFSET requires a number".into(),
            ));
        };
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError(
                "derived grouped LIMIT/OFFSET must be non-negative".into(),
            ));
        }
        value
            .parse::<usize>()
            .map(Some)
            .map_err(|_| ParseError("derived grouped LIMIT/OFFSET is too large".into()))
    };
    let limit = parse_bound(limit_index)?;
    let offset = parse_bound(offset_index)?.unwrap_or(0);
    if tokens
        .iter()
        .enumerate()
        .any(|(index, token)| index > close && keyword(token, "FETCH"))
    {
        return Ok(None);
    }
    Ok(Some(DerivedGroupedSelect {
        inner,
        columns,
        filter: DerivedGroupedFilter::WindowDeviation {
            average_index,
            sum_index,
            threshold,
        },
        order,
        limit,
        offset,
    }))
}

/// Parse an outer join over one grouped COUNT(*) derived table.
pub fn parse_derived_grouped_join_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<DerivedColumn>, ParseError>,
) -> Result<Option<DerivedGroupedJoin>, ParseError> {
    let tokens = tokenize(sql)?;
    let Some((from, close)) = derived_bounds(&tokens) else {
        return Ok(None);
    };
    let inner_sql = render_tokens(&tokens[from + 2..close]);
    let Some(inner) = parse_join_with_schema(&inner_sql, |table| {
        resolve(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
    })?
    else {
        return Ok(None);
    };
    let Some(columns) = grouped_select_columns(&inner, &mut resolve)? else {
        return Ok(None);
    };

    let mut outer_tokens = tokens[..from + 1].to_vec();
    outer_tokens.push(Token::Word(DERIVED_TABLE.into()));
    // Keep the derived-table alias and the rest of the outer query intact.
    outer_tokens.extend(expand_simple_between(&tokens[close + 1..]));
    let outer_sql = render_tokens(&outer_tokens);
    let outer = match super::parse_derived_outer_join_with_schema(&outer_sql, |table| {
        if table.eq_ignore_ascii_case(DERIVED_TABLE) {
            Ok(columns.iter().map(|column| column.name.clone()).collect())
        } else {
            resolve(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
        }
    })? {
        Some(outer) => outer,
        None => return Ok(None),
    };
    if outer.columns.is_none()
        || outer.aggregate.is_some()
        || !outer.scalar_aggregates.is_empty()
        || outer.grouped_count.is_some()
        || outer.scalar_sum.is_some()
        || outer.scalar_formula.is_some()
    {
        return Ok(None);
    }
    Ok(Some(DerivedGroupedJoin {
        inner,
        outer,
        table: DERIVED_TABLE.into(),
        columns,
    }))
}

/// A bounded UNION ALL source whose branches are simple joined projections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedUnionJoin {
    pub(crate) branches: Vec<JoinSelect>,
    pub(crate) outer: JoinSelect,
    pub(crate) table: String,
    pub(crate) columns: Vec<DerivedColumn>,
}

impl DerivedUnionJoin {
    pub fn branches(&self) -> &[JoinSelect] {
        &self.branches
    }

    pub fn outer(&self) -> &JoinSelect {
        &self.outer
    }

    pub fn table(&self) -> &str {
        &self.table
    }

    pub fn columns(&self) -> &[DerivedColumn] {
        &self.columns
    }

    pub fn cap_rows(&mut self, cap: usize) {
        self.outer.limit = Some(self.outer.limit.map_or(cap, |limit| limit.min(cap)));
    }
}

/// Detect a derived source containing multiple top-level UNION ALL branches.
pub fn has_derived_union(sql: &str) -> Result<bool, ParseError> {
    let tokens = tokenize(sql)?;
    let Some((open, close)) = union_derived_bounds(&tokens) else {
        return Ok(false);
    };
    Ok(split_union_all(&tokens[open + 1..close]).is_some())
}

/// Parse an outer grouped join over a bounded UNION ALL of joined projections.
pub fn parse_derived_union_join_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<DerivedColumn>, ParseError>,
) -> Result<Option<DerivedUnionJoin>, ParseError> {
    let tokens = tokenize(sql)?;
    let Some((open, close)) = union_derived_bounds(&tokens) else {
        return Ok(None);
    };
    let Some(branch_tokens) = split_union_all(&tokens[open + 1..close]) else {
        return Ok(None);
    };

    let mut branches = Vec::with_capacity(branch_tokens.len());
    let mut output_columns: Option<Vec<DerivedColumn>> = None;
    for branch in branch_tokens {
        let branch_sql = render_tokens(branch);
        let Some(parsed) = parse_join_with_schema(&branch_sql, |table| {
            resolve(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
        })?
        else {
            return Ok(None);
        };
        if parsed.kind != JoinType::Inner
            || parsed.steps.iter().any(|step| step.kind != JoinType::Inner)
            || parsed.columns.is_none()
            || parsed.aggregate.is_some()
            || !parsed.scalar_aggregates.is_empty()
            || parsed.grouped_count.is_some()
            || parsed.grouped_keys_only
            || parsed.grouped_count_column.is_some()
            || !parsed.grouped_case_counts.is_empty()
            || parsed.grouped_sum.is_some()
            || !parsed.grouped_sum_extras.is_empty()
            || parsed.grouped_min.is_some()
            || !parsed.grouped_avg_metrics.is_empty()
            || parsed.grouped_formula.is_some()
            || parsed.grouped_rollup
            || parsed.grouped_year.is_some()
            || parsed.grouped_window_ratio.is_some()
            || parsed.grouped_sum_having.is_some()
            || parsed.grouped_count_having.is_some()
            || parsed.scalar_sum.is_some()
            || parsed.scalar_formula.is_some()
            || parsed.grouped_distinct.is_some()
            || parsed.in_subquery.is_some()
            || parsed.limit.is_some()
            || parsed.offset != 0
            || !parsed.order_by.is_empty()
        {
            return Ok(None);
        }
        let projected = parsed.columns.as_ref().expect("simple projection");
        if projected.is_empty() || projected.iter().any(|column| column.name == "*") {
            return Ok(None);
        }
        let mut columns = Vec::with_capacity(projected.len());
        for (position, column) in projected.iter().enumerate() {
            let Some(data_type) = source_type(&parsed, column, &mut resolve)? else {
                return Ok(None);
            };
            let name = parsed
                .grouped_key_aliases
                .iter()
                .find_map(|(index, alias)| (*index == position).then(|| alias.clone()))
                .unwrap_or_else(|| column.name.clone());
            if columns
                .iter()
                .any(|previous: &DerivedColumn| previous.name.eq_ignore_ascii_case(&name))
            {
                return Ok(None);
            }
            columns.push(DerivedColumn { name, data_type });
        }
        if let Some(expected) = &output_columns {
            if expected.len() != columns.len()
                || expected
                    .iter()
                    .zip(&columns)
                    .any(|(left, right)| left.data_type != right.data_type)
            {
                return Ok(None);
            }
        } else {
            output_columns = Some(columns);
        }
        branches.push(parsed);
    }
    let columns = output_columns.expect("at least two UNION ALL branches");

    let mut outer_tokens = tokens[..open].to_vec();
    outer_tokens.push(Token::Word(DERIVED_TABLE.into()));
    outer_tokens.extend_from_slice(&tokens[close + 1..]);
    let outer_sql = render_tokens(&outer_tokens);
    let outer = match super::parse_derived_outer_join_with_schema(&outer_sql, |table| {
        if table.eq_ignore_ascii_case(DERIVED_TABLE) {
            Ok(columns.iter().map(|column| column.name.clone()).collect())
        } else {
            resolve(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
        }
    })? {
        Some(outer) => outer,
        None => return Ok(None),
    };
    let derived_references = std::iter::once(&outer.left)
        .chain(std::iter::once(&outer.right))
        .chain(outer.steps.iter().map(|step| &step.table))
        .filter(|table| table.eq_ignore_ascii_case(DERIVED_TABLE))
        .count();
    if outer.columns.is_none() || outer.grouped_sum.is_none() || derived_references != 1 {
        return Ok(None);
    }
    Ok(Some(DerivedUnionJoin {
        branches,
        outer,
        table: DERIVED_TABLE.into(),
        columns,
    }))
}
