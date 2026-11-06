//! Materialized CTEs whose inputs and final query are supported JOIN plans.

use crate::{
    GroupedSum, JoinColumn, JoinSelect, ParseError, Parser, Token, parse_join_with_schema, tokenize,
};

use super::cte_correlated_avg::CteOutputColumn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CteMultiJoinSource {
    name: String,
    query: CteSourceQuery,
    columns: Vec<CteOutputColumn>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CteSourceQuery {
    Join(Box<JoinSelect>),
    Union(Box<crate::DerivedUnionSelect>),
}

type CteDefinitions = Vec<(String, String)>;

impl CteMultiJoinSource {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn query(&self) -> &CteSourceQuery {
        &self.query
    }

    pub fn columns(&self) -> &[CteOutputColumn] {
        &self.columns
    }
}

/// A set of materialized JOIN CTEs followed by a JOIN query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CteMultiJoin {
    sources: Vec<CteMultiJoinSource>,
    outer: JoinSelect,
}

impl CteMultiJoin {
    pub fn sources(&self) -> &[CteMultiJoinSource] {
        &self.sources
    }

    pub fn outer(&self) -> &JoinSelect {
        &self.outer
    }

    pub fn cap_rows(&mut self, cap: usize) {
        self.outer.limit = Some(self.outer.limit.map_or(cap, |limit| limit.min(cap)));
    }
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn render_token(token: &Token) -> String {
    match token {
        Token::Word(value) | Token::Number(value) => value.clone(),
        Token::QuotedWord(value) | Token::QuotedAlias(value) => quote_identifier(value),
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
            crate::Operator::Eq => "=".into(),
            crate::Operator::Ne => "!=".into(),
            crate::Operator::Lt => "<".into(),
            crate::Operator::Le => "<=".into(),
            crate::Operator::Gt => ">".into(),
            crate::Operator::Ge => ">=".into(),
            crate::Operator::Modulo(value) => format!("%{value}"),
            crate::Operator::ModuloNe(value) => format!("%{value} !="),
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

fn identifier_token(token: &Token) -> bool {
    matches!(token, Token::Word(_) | Token::QuotedWord(_))
}

fn is_column_tokens(tokens: &[Token]) -> bool {
    matches!(tokens, [name] if identifier_token(name))
        || matches!(tokens, [relation, Token::Dot, name] if identifier_token(relation) && identifier_token(name))
}

fn has_column_offset_equality(tokens: &[Token]) -> bool {
    for left_len in [1, 3] {
        for right_len in [1, 3] {
            let width = left_len + 1 + right_len + 2;
            for comparison in tokens.windows(width) {
                let right_start = left_len + 1;
                if is_column_tokens(&comparison[..left_len])
                    && comparison[left_len] == Token::Operator(crate::Operator::Eq)
                    && is_column_tokens(&comparison[right_start..right_start + right_len])
                    && matches!(
                        comparison[right_start + right_len],
                        Token::Plus | Token::Minus
                    )
                    && matches!(&comparison[right_start + right_len + 1], Token::Number(value) if value.bytes().all(|byte| byte.is_ascii_digit()))
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Use a bounded CROSS JOIN for a two-source offset equality that the key
/// planner cannot normalize into a hash key. The WHERE expression still checks
/// every candidate pair, and normal JOIN result limits remain in force.
fn rewrite_offset_comma_join(sql: &str) -> Result<Option<String>, ParseError> {
    let tokens = tokenize(sql)?;
    let mut depth = 0usize;
    let mut from = None;
    let mut where_clause = None;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("FROM") => {
                from = Some(index);
            }
            Token::Word(word)
                if depth == 0 && from.is_some() && word.eq_ignore_ascii_case("WHERE") =>
            {
                where_clause = Some(index);
                break;
            }
            _ => {}
        }
    }
    let (Some(from), Some(where_clause)) = (from, where_clause) else {
        return Ok(None);
    };
    if !has_column_offset_equality(&tokens[where_clause + 1..]) {
        return Ok(None);
    }

    depth = 0;
    let mut comma = None;
    for (index, token) in tokens.iter().enumerate().take(where_clause).skip(from + 1) {
        match token {
            Token::LParen => return Ok(None),
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Comma if depth == 0 => {
                if comma.replace(index).is_some() {
                    return Ok(None);
                }
            }
            Token::Word(word)
                if depth == 0
                    && ["JOIN", "CROSS", "LEFT", "RIGHT", "FULL"]
                        .iter()
                        .any(|keyword| word.eq_ignore_ascii_case(keyword)) =>
            {
                return Ok(None);
            }
            _ => {}
        }
    }
    let Some(comma) = comma else {
        return Ok(None);
    };
    let mut rewritten = tokens[..comma].to_vec();
    rewritten.extend([Token::Word("CROSS".into()), Token::Word("JOIN".into())]);
    rewritten.extend_from_slice(&tokens[comma + 1..]);
    Ok(Some(render_tokens(&rewritten)))
}

fn split_ctes(sql: &str) -> Result<Option<(CteDefinitions, String)>, ParseError> {
    let tokens = tokenize(sql)?;
    let mut parser = Parser { tokens, index: 0 };
    if !parser.keyword("WITH") {
        return Ok(None);
    }

    let mut sources = Vec::new();
    loop {
        let Ok(name) = parser.bounded_word() else {
            return Ok(None);
        };
        if sources
            .iter()
            .any(|(existing, _): &(String, String)| existing.eq_ignore_ascii_case(&name))
        {
            return Ok(None);
        }
        if !parser.keyword("AS") || !parser.take(Token::LParen) {
            return Ok(None);
        }

        let start = parser.index;
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
            return Ok(None);
        }
        sources.push((name, render_tokens(&parser.tokens[start..parser.index])));
        parser.index += 1;
        if !parser.take(Token::Comma) {
            break;
        }
    }

    if !matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT"))
    {
        return Ok(None);
    }
    let outer = render_tokens(&parser.tokens[parser.index..]);
    Ok(Some((sources, outer)))
}

fn source_name(token: &Token) -> Option<String> {
    match token {
        Token::Word(name) | Token::QuotedWord(name) | Token::QuotedAlias(name) => {
            Some(name.clone())
        }
        _ => None,
    }
}

fn derived_source(tokens: &[Token], start: usize) -> Option<(String, String, usize)> {
    if tokens.get(start) != Some(&Token::LParen) {
        return None;
    }
    let mut depth = 1usize;
    let mut close = start + 1;
    while close < tokens.len() && depth > 0 {
        match tokens[close] {
            Token::LParen => depth += 1,
            Token::RParen => depth -= 1,
            _ => {}
        }
        close += 1;
    }
    if depth != 0
        || !matches!(tokens.get(start + 1), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT"))
    {
        return None;
    }
    let mut alias_index = close;
    if matches!(tokens.get(alias_index), Some(Token::Word(word)) if word.eq_ignore_ascii_case("AS"))
    {
        alias_index += 1;
    }
    let alias = source_name(tokens.get(alias_index)?)?;
    let body = render_tokens(&tokens[start + 1..close - 1]);
    Some((alias, body, alias_index + 1))
}

/// Replace two top-level derived tables with names so they can be materialized in order.
fn split_two_derived_sources(tokens: &[Token]) -> Option<(Vec<(String, String)>, String)> {
    let mut depth = 0usize;
    let from = tokens.iter().position(|token| match token {
        Token::LParen => {
            depth += 1;
            false
        }
        Token::RParen => {
            depth = depth.saturating_sub(1);
            false
        }
        Token::Word(word) => depth == 0 && word.eq_ignore_ascii_case("FROM"),
        _ => false,
    })?;
    let (left_name, left_query, after_left) = derived_source(tokens, from + 1)?;
    if tokens.get(after_left) != Some(&Token::Comma) {
        return None;
    }
    let (right_name, right_query, after_right) = derived_source(tokens, after_left + 1)?;
    if left_name.eq_ignore_ascii_case(&right_name)
        || matches!(tokens.get(after_right), Some(Token::Comma))
    {
        return None;
    }

    let mut rewritten = tokens[..from + 1].to_vec();
    rewritten.push(Token::Word(left_name.clone()));
    rewritten.push(Token::Comma);
    rewritten.push(Token::Word(right_name.clone()));
    rewritten.extend_from_slice(&tokens[after_right..]);
    Some((
        vec![(left_name, left_query), (right_name, right_query)],
        render_tokens(&rewritten),
    ))
}

fn split_union_all(tokens: &[Token]) -> Option<Vec<&[Token]>> {
    let mut branches = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut index = 0usize;
    while index < tokens.len() {
        match tokens[index] {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(ref word)
                if depth == 0
                    && word.eq_ignore_ascii_case("UNION")
                    && matches!(tokens.get(index + 1), Some(Token::Word(all)) if all.eq_ignore_ascii_case("ALL")) =>
            {
                if start == index {
                    return None;
                }
                branches.push(&tokens[start..index]);
                index += 1;
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    if branches.is_empty() || start >= tokens.len() {
        return None;
    }
    branches.push(&tokens[start..]);
    Some(branches)
}

fn cte_union_bounds(tokens: &[Token]) -> Option<(usize, usize)> {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(word)
                if depth == 0
                    && word.eq_ignore_ascii_case("FROM")
                    && tokens.get(index + 1) == Some(&Token::LParen) =>
            {
                let open = index + 1;
                let mut nested = 1usize;
                let mut close = open + 1;
                while close < tokens.len() && nested > 0 {
                    match tokens[close] {
                        Token::LParen => nested += 1,
                        Token::RParen => nested -= 1,
                        _ => {}
                    }
                    close += 1;
                }
                if nested == 0 && split_union_all(&tokens[open + 1..close - 1]).is_some() {
                    return Some((open, close - 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse grouped CTEs combined by UNION ALL and consumed by one grouped SELECT.
/// Each union branch must select all columns from one grouped CTE.
pub fn parse_cte_union_select_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<crate::DerivedColumn>, ParseError>,
) -> Result<Option<crate::DerivedUnionSelect>, ParseError> {
    let Some((definitions, outer_sql)) = split_ctes(sql)? else {
        return Ok(None);
    };
    let outer_tokens = tokenize(&outer_sql)?;
    let Some((open, close)) = cte_union_bounds(&outer_tokens) else {
        return Ok(None);
    };
    let Some(branches) = split_union_all(&outer_tokens[open + 1..close]) else {
        return Ok(None);
    };

    let mut expanded = outer_tokens[..open + 1].to_vec();
    for (index, branch) in branches.into_iter().enumerate() {
        let [
            Token::Word(select),
            Token::Star,
            Token::Word(from),
            Token::Word(name),
        ] = branch
        else {
            return Ok(None);
        };
        if !select.eq_ignore_ascii_case("SELECT") || !from.eq_ignore_ascii_case("FROM") {
            return Ok(None);
        }
        let Some((_, body)) = definitions
            .iter()
            .find(|(defined, _)| defined.eq_ignore_ascii_case(name))
        else {
            return Ok(None);
        };
        let Ok(body_tokens) = tokenize(body) else {
            return Ok(None);
        };
        if index > 0 {
            expanded.extend([Token::Word("UNION".into()), Token::Word("ALL".into())]);
        }
        expanded.extend(body_tokens);
    }
    expanded.push(Token::RParen);
    expanded.extend_from_slice(&outer_tokens[close + 1..]);

    let expanded_sql = render_tokens(&expanded);
    let Ok(Some(plan)) = crate::parse_derived_union_select_with_schema(&expanded_sql, &mut resolve)
    else {
        return Ok(None);
    };
    Ok(Some(plan))
}

fn table_sources(query: &JoinSelect) -> Vec<(&str, &str)> {
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
    mut resolve: impl FnMut(&str) -> Result<Vec<CteOutputColumn>, ParseError>,
) -> Result<Option<String>, ParseError> {
    let mut found = None;
    for (table, alias) in table_sources(query) {
        if !column.relation.is_empty()
            && !column.relation.eq_ignore_ascii_case(alias)
            && !column.relation.eq_ignore_ascii_case(table)
        {
            continue;
        }
        let schema = resolve(table)?;
        if let Some(metadata) = schema
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(&column.name))
        {
            if found.replace(metadata.data_type.clone()).is_some() {
                return Ok(None);
            }
        }
    }
    Ok(found)
}

fn grouped_sum_type(
    query: &JoinSelect,
    metric: &GroupedSum,
    resolve: &mut impl FnMut(&str) -> Result<Vec<CteOutputColumn>, ParseError>,
) -> Result<Option<String>, ParseError> {
    let data_type = match metric {
        GroupedSum::Column(column) => {
            let Some(data_type) = source_type(query, column, resolve)? else {
                return Ok(None);
            };
            data_type
        }
        GroupedSum::Expression(_) => return Ok(Some("Nullable(Float64)".into())),
    };
    let base_type = data_type
        .strip_prefix("Nullable(")
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(&data_type);
    let result_type = match base_type {
        "Float32" | "Float64" => "Nullable(Float64)",
        "Int8" | "Int16" | "Int32" | "Int64" | "Int128" => "Nullable(Int128)",
        "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128" => "Nullable(UInt128)",
        _ => return Ok(None),
    };
    Ok(Some(result_type.into()))
}

fn output_schema(
    query: &JoinSelect,
    mut resolve: impl FnMut(&str) -> Result<Vec<CteOutputColumn>, ParseError>,
) -> Result<Option<Vec<CteOutputColumn>>, ParseError> {
    let Some(keys) = query.columns.as_ref() else {
        return Ok(None);
    };
    if keys.is_empty()
        || keys.iter().any(|key| key.name == "*")
        || query.aggregate.is_some()
        || !query.scalar_aggregates.is_empty()
        || query.scalar_sum.is_some()
        || query.scalar_formula.is_some()
        || query.grouped_count_column.is_some()
        || query.grouped_distinct.is_some()
        || query.grouped_min.is_some()
        || !query.grouped_min_metrics.is_empty()
        || !query.grouped_max_metrics.is_empty()
        || !query.grouped_avg_metrics.is_empty()
        || query.grouped_window_average.is_some()
        || query.grouped_window_ratio.is_some()
        || query.grouped_formula.is_some()
        || query.grouped_year.is_some()
        || query.grouped_sum_having.is_some()
        || query.grouped_count_having.is_some()
        || !query.scalar_projections.is_empty()
    {
        return Ok(None);
    }

    if !query.grouped_case_counts.is_empty() {
        let (Some(grouped_keys), Some(keys)) =
            (query.grouped_count.as_ref(), query.columns.as_ref())
        else {
            return Ok(None);
        };
        if keys.is_empty()
            || keys.len() != grouped_keys.len()
            || query.grouped_rollup
            || query.grouped_sum.is_some()
            || !query.grouped_sum_extras.is_empty()
            || !matches!(query.kind, crate::JoinType::Inner)
            || query
                .steps
                .iter()
                .any(|step| step.kind != crate::JoinType::Inner)
        {
            return Ok(None);
        }

        let mut columns = Vec::with_capacity(keys.len() + query.grouped_case_counts.len());
        for (index, key) in keys.iter().enumerate() {
            let Some(data_type) = source_type(query, key, &mut resolve)? else {
                return Ok(None);
            };
            let name = query
                .grouped_key_aliases
                .iter()
                .find_map(|(key_index, alias)| (*key_index == index).then(|| alias.clone()))
                .unwrap_or_else(|| key.name.clone());
            columns.push(CteOutputColumn { name, data_type });
        }
        for (_, value, alias) in &query.grouped_case_counts {
            let data_type = match value {
                crate::GroupedCaseValue::Count => "Nullable(UInt64)".into(),
                crate::GroupedCaseValue::Column(column) => {
                    let Some(data_type) =
                        grouped_sum_type(query, &GroupedSum::Column(column.clone()), &mut resolve)?
                    else {
                        return Ok(None);
                    };
                    data_type
                }
                crate::GroupedCaseValue::Expression(expression) => {
                    let Some(data_type) = grouped_sum_type(
                        query,
                        &GroupedSum::Expression(expression.clone()),
                        &mut resolve,
                    )?
                    else {
                        return Ok(None);
                    };
                    data_type
                }
            };
            columns.push(CteOutputColumn {
                name: alias.clone(),
                data_type,
            });
        }
        if columns.iter().enumerate().any(|(index, column)| {
            columns[..index]
                .iter()
                .any(|previous| previous.name.eq_ignore_ascii_case(&column.name))
        }) {
            return Ok(None);
        }
        return Ok(Some(columns));
    }

    let has_sum = query.grouped_sum.is_some();
    if has_sum {
        let (Some(grouped_keys), Some(metric)) =
            (query.grouped_count.as_ref(), query.grouped_sum.as_ref())
        else {
            return Ok(None);
        };
        if keys.len() != grouped_keys.len()
            || query.grouped_rollup
            || !matches!(query.kind, crate::JoinType::Inner | crate::JoinType::Left)
            || query
                .steps
                .iter()
                .any(|step| !matches!(step.kind, crate::JoinType::Inner | crate::JoinType::Left))
        {
            return Ok(None);
        }
        if query.scalar_sum.is_some()
            || query.scalar_formula.is_some()
            || query.grouped_sum_having.is_some()
            || query.grouped_count_having.is_some()
            || query.grouped_output.is_empty()
        {
            return Ok(None);
        }

        let mut columns = Vec::with_capacity(keys.len() + 1 + query.grouped_sum_extras.len());
        for item in &query.grouped_output {
            let column = match item {
                crate::JoinGroupedOutput::Key(index) => {
                    let Some(key) = keys.get(*index) else {
                        return Ok(None);
                    };
                    let Some(data_type) = source_type(query, key, &mut resolve)? else {
                        return Ok(None);
                    };
                    let name = query
                        .grouped_key_aliases
                        .iter()
                        .find_map(|(key_index, alias)| {
                            (*key_index == *index).then(|| alias.clone())
                        })
                        .unwrap_or_else(|| key.name.clone());
                    CteOutputColumn { name, data_type }
                }
                crate::JoinGroupedOutput::Sum(name) => {
                    let Some(data_type) = grouped_sum_type(query, metric, &mut resolve)? else {
                        return Ok(None);
                    };
                    CteOutputColumn {
                        name: name.clone(),
                        data_type,
                    }
                }
                crate::JoinGroupedOutput::SumExtra(index, name) => {
                    let Some((metric, _)) = query.grouped_sum_extras.get(*index) else {
                        return Ok(None);
                    };
                    let Some(data_type) = grouped_sum_type(query, metric, &mut resolve)? else {
                        return Ok(None);
                    };
                    CteOutputColumn {
                        name: name.clone(),
                        data_type,
                    }
                }
                _ => return Ok(None),
            };
            if columns
                .iter()
                .any(|existing: &CteOutputColumn| existing.name.eq_ignore_ascii_case(&column.name))
            {
                return Ok(None);
            }
            columns.push(column);
        }
        if columns.len() != keys.len() + 1 + query.grouped_sum_extras.len() {
            return Ok(None);
        }
        return Ok(Some(columns));
    }

    if query.grouped_keys_only {
        let mut columns = Vec::with_capacity(keys.len());
        for (position, key) in keys.iter().enumerate() {
            let Some(data_type) = source_type(query, key, &mut resolve)? else {
                return Ok(None);
            };
            let name = query
                .grouped_key_aliases
                .iter()
                .find_map(|(index, alias)| (*index == position).then(|| alias.clone()))
                .unwrap_or_else(|| key.name.clone());
            if columns
                .iter()
                .any(|column: &CteOutputColumn| column.name.eq_ignore_ascii_case(&name))
            {
                return Ok(None);
            }
            columns.push(CteOutputColumn { name, data_type });
        }
        return Ok(Some(columns));
    }

    if query.grouped_count.is_some()
        || query.grouped_sum.is_some()
        || !query.grouped_sum_extras.is_empty()
        || query.grouped_rollup
        || query.limit.is_some()
        || query.offset != 0
        || !query.order_by.is_empty()
    {
        return Ok(None);
    }

    let mut columns = Vec::with_capacity(keys.len());
    for (position, key) in keys.iter().enumerate() {
        let Some(data_type) = source_type(query, key, &mut resolve)? else {
            return Ok(None);
        };
        let name = query
            .grouped_key_aliases
            .iter()
            .find_map(|(index, alias)| (*index == position).then(|| alias.clone()))
            .unwrap_or_else(|| key.name.clone());
        if columns
            .iter()
            .any(|column: &CteOutputColumn| column.name.eq_ignore_ascii_case(&name))
        {
            return Ok(None);
        }
        columns.push(CteOutputColumn { name, data_type });
    }
    Ok(Some(columns))
}

fn union_output_schema(query: &crate::DerivedUnionSelect) -> Option<Vec<CteOutputColumn>> {
    let input_columns = query.columns();
    let selected = match &query.outer().projection {
        crate::Projection::All => input_columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>(),
        crate::Projection::Columns(selected) => selected.clone(),
        _ => return None,
    };
    if selected.len() != input_columns.len()
        || query.outer().rename.is_some()
        || selected
            .iter()
            .zip(input_columns)
            .any(|(selected, input)| !selected.eq_ignore_ascii_case(&input.name))
    {
        return None;
    }
    Some(
        selected
            .iter()
            .zip(input_columns)
            .map(|(name, input)| CteOutputColumn {
                name: name.clone(),
                data_type: input.data_type.clone(),
            })
            .collect(),
    )
}

fn resolve_derived_columns(
    table: &str,
    sources: &[CteMultiJoinSource],
    resolve: &mut impl FnMut(&str) -> Result<Vec<CteOutputColumn>, ParseError>,
) -> Result<Vec<crate::DerivedColumn>, ParseError> {
    if let Some(source) = sources
        .iter()
        .find(|source| source.name.eq_ignore_ascii_case(table))
    {
        Ok(source
            .columns
            .iter()
            .map(|column| crate::DerivedColumn {
                name: column.name.clone(),
                data_type: column.data_type.clone(),
            })
            .collect())
    } else {
        resolve(table).map(|columns| {
            columns
                .into_iter()
                .map(|column| crate::DerivedColumn {
                    name: column.name,
                    data_type: column.data_type,
                })
                .collect()
        })
    }
}

/// Parse one or more CTE JOIN plans followed by a supported JOIN query.
pub fn parse_cte_multi_join_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<CteOutputColumn>, ParseError>,
) -> Result<Option<CteMultiJoin>, ParseError> {
    let Some((definitions, mut outer_sql)) = split_ctes(sql)? else {
        return Ok(None);
    };
    let mut sources: Vec<CteMultiJoinSource> = Vec::with_capacity(definitions.len());
    for (name, body) in definitions {
        let parse_result = parse_join_with_schema(&body, |table| {
            if let Some(source) = sources
                .iter()
                .find(|source| source.name.eq_ignore_ascii_case(table))
            {
                Ok(source
                    .columns
                    .iter()
                    .map(|column| column.name.clone())
                    .collect())
            } else {
                resolve(table)
                    .map(|columns| columns.into_iter().map(|column| column.name).collect())
            }
        });
        if let Ok(Some(query)) = parse_result {
            let schema_result = output_schema(&query, |table| {
                if let Some(source) = sources
                    .iter()
                    .find(|source| source.name.eq_ignore_ascii_case(table))
                {
                    Ok(source.columns.clone())
                } else {
                    resolve(table)
                }
            });
            if let Ok(Some(columns)) = schema_result {
                sources.push(CteMultiJoinSource {
                    name,
                    query: CteSourceQuery::Join(Box::new(query)),
                    columns,
                });
                continue;
            }
        }

        let direct_union = crate::parse_derived_union_select_with_schema(&body, |table| {
            resolve_derived_columns(table, &sources, &mut resolve)
        })?;
        let query = if let Some(query) = direct_union {
            query
        } else {
            let wrapped = format!("SELECT * FROM ({body}) __cte_union");
            let union_result = crate::parse_derived_union_select_with_schema(&wrapped, |table| {
                resolve_derived_columns(table, &sources, &mut resolve)
            })?;
            let Some(query) = union_result else {
                return Ok(None);
            };
            query
        };
        let Some(columns) = union_output_schema(&query) else {
            return Ok(None);
        };
        sources.push(CteMultiJoinSource {
            name,
            query: CteSourceQuery::Union(Box::new(query)),
            columns,
        });
    }

    if let Some((derived_sources, rewritten_outer)) =
        split_two_derived_sources(&tokenize(&outer_sql)?)
    {
        for (name, body) in derived_sources {
            if sources
                .iter()
                .any(|source| source.name.eq_ignore_ascii_case(&name))
            {
                return Ok(None);
            }
            let parse_result = parse_join_with_schema(&body, |table| {
                if let Some(source) = sources
                    .iter()
                    .find(|source| source.name.eq_ignore_ascii_case(table))
                {
                    Ok(source
                        .columns
                        .iter()
                        .map(|column| column.name.clone())
                        .collect())
                } else {
                    resolve(table)
                        .map(|columns| columns.into_iter().map(|column| column.name).collect())
                }
            });
            let Ok(Some(query)) = parse_result else {
                return Ok(None);
            };
            let schema_result = output_schema(&query, |table| {
                if let Some(source) = sources
                    .iter()
                    .find(|source| source.name.eq_ignore_ascii_case(table))
                {
                    Ok(source.columns.clone())
                } else {
                    resolve(table)
                }
            });
            let Ok(Some(columns)) = schema_result else {
                return Ok(None);
            };
            sources.push(CteMultiJoinSource {
                name,
                query: CteSourceQuery::Join(Box::new(query)),
                columns,
            });
        }
        outer_sql = rewritten_outer;
    }

    let parse_result = parse_join_with_schema(&outer_sql, |table| {
        if let Some(source) = sources
            .iter()
            .find(|source| source.name.eq_ignore_ascii_case(table))
        {
            Ok(source
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect())
        } else {
            resolve(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
        }
    });
    let outer = match parse_result {
        Ok(Some(outer)) => outer,
        Err(error) if error.0.contains("comma joins require a connected graph") => {
            let Some(rewritten) = rewrite_offset_comma_join(&outer_sql)? else {
                return Ok(None);
            };
            let Ok(Some(outer)) = parse_join_with_schema(&rewritten, |table| {
                if let Some(source) = sources
                    .iter()
                    .find(|source| source.name.eq_ignore_ascii_case(table))
                {
                    Ok(source
                        .columns
                        .iter()
                        .map(|column| column.name.clone())
                        .collect())
                } else {
                    resolve(table)
                        .map(|columns| columns.into_iter().map(|column| column.name).collect())
                }
            }) else {
                return Ok(None);
            };
            outer
        }
        _ => return Ok(None),
    };
    if outer.limit.is_none() && outer.offset != 0 {
        return Ok(None);
    }

    let mut needed = Vec::new();
    for table in [&outer.left, &outer.right]
        .into_iter()
        .chain(outer.steps.iter().map(|step| &step.table))
    {
        if sources
            .iter()
            .any(|source| source.name.eq_ignore_ascii_case(table))
            && !needed
                .iter()
                .any(|name: &String| name.eq_ignore_ascii_case(table))
        {
            needed.push(table.clone());
        }
    }
    loop {
        let before = needed.len();
        for source in &sources {
            if !needed
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&source.name))
            {
                continue;
            }
            let mut add = |table: &str| {
                if sources.iter().any(|candidate| {
                    candidate.name.eq_ignore_ascii_case(table)
                        && !needed.iter().any(|name| name.eq_ignore_ascii_case(table))
                }) {
                    needed.push(table.to_owned());
                }
            };
            match &source.query {
                CteSourceQuery::Join(query) => {
                    for table in [&query.left, &query.right]
                        .into_iter()
                        .chain(query.steps.iter().map(|step| &step.table))
                    {
                        add(table);
                    }
                }
                CteSourceQuery::Union(query) => {
                    for table in query
                        .branches()
                        .iter()
                        .flat_map(|branch| branch.source_tables())
                    {
                        add(table);
                    }
                }
            }
        }
        if needed.len() == before {
            break;
        }
    }
    sources.retain(|source| {
        needed
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&source.name))
    });
    Ok(Some(CteMultiJoin { sources, outer }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(table: &str) -> Result<Vec<CteOutputColumn>, ParseError> {
        let columns = match table {
            "store_sales" => vec![
                ("ss_addr_sk", "Nullable(Int64)"),
                ("ss_customer_sk", "Nullable(Int64)"),
                ("ss_item_sk", "Nullable(Int64)"),
                ("ss_quantity", "Nullable(Int64)"),
                ("ss_store_sk", "Nullable(Int64)"),
                ("ss_sold_date_sk", "Nullable(Int64)"),
                ("ss_ext_sales_price", "Nullable(Float64)"),
                ("ss_sales_price", "Nullable(Float64)"),
                ("ss_wholesale_cost", "Nullable(Float64)"),
                ("ss_ticket_number", "Nullable(Int64)"),
            ],
            "store_returns" => vec![
                ("sr_item_sk", "Nullable(Int64)"),
                ("sr_ticket_number", "Nullable(Int64)"),
            ],
            "web_sales" => vec![
                ("ws_addr_sk", "Nullable(Int64)"),
                ("ws_bill_customer_sk", "Nullable(Int64)"),
                ("ws_item_sk", "Nullable(Int64)"),
                ("ws_order_number", "Nullable(Int64)"),
                ("ws_quantity", "Nullable(Int64)"),
                ("ws_sold_date_sk", "Nullable(Int64)"),
                ("ws_ext_sales_price", "Nullable(Float64)"),
                ("ws_sales_price", "Nullable(Float64)"),
                ("ws_wholesale_cost", "Nullable(Float64)"),
            ],
            "web_returns" => vec![
                ("wr_item_sk", "Nullable(Int64)"),
                ("wr_order_number", "Nullable(Int64)"),
            ],
            "catalog_sales" => vec![
                ("cs_bill_customer_sk", "Nullable(Int64)"),
                ("cs_item_sk", "Nullable(Int64)"),
                ("cs_order_number", "Nullable(Int64)"),
                ("cs_quantity", "Nullable(Int64)"),
                ("cs_sold_date_sk", "Nullable(Int64)"),
                ("cs_ext_sales_price", "Nullable(Float64)"),
                ("cs_sales_price", "Nullable(Float64)"),
                ("cs_wholesale_cost", "Nullable(Float64)"),
            ],
            "catalog_returns" => vec![
                ("cr_item_sk", "Nullable(Int64)"),
                ("cr_order_number", "Nullable(Int64)"),
            ],
            "date_dim" => vec![
                ("d_date_sk", "Nullable(Int64)"),
                ("d_week_seq", "Nullable(Int64)"),
                ("d_day_name", "Nullable(String)"),
                ("d_qoy", "Nullable(Int64)"),
                ("d_year", "Nullable(Int64)"),
                ("d_month_seq", "Nullable(Int64)"),
            ],
            "store" => vec![
                ("s_store_sk", "Nullable(Int64)"),
                ("s_store_name", "Nullable(String)"),
                ("s_store_id", "Nullable(String)"),
            ],
            "customer_address" => vec![
                ("ca_address_sk", "Nullable(Int64)"),
                ("ca_county", "Nullable(String)"),
            ],
            _ => return Err(ParseError(format!("unknown test table {table}"))),
        };
        Ok(columns
            .into_iter()
            .map(|(name, data_type)| CteOutputColumn {
                name: name.into(),
                data_type: data_type.into(),
            })
            .collect())
    }

    #[test]
    fn parses_multiple_grouped_sum_ctes_and_outer_join() {
        let sql = "WITH ss AS (SELECT ca_county, d_qoy, d_year, SUM(ss_ext_sales_price) AS store_sales FROM store_sales, date_dim, customer_address WHERE ss_sold_date_sk = d_date_sk AND ss_addr_sk = ca_address_sk GROUP BY ca_county, d_qoy, d_year), ws AS (SELECT ca_county, d_qoy, d_year, SUM(ws_ext_sales_price) AS web_sales FROM web_sales, date_dim, customer_address WHERE ws_sold_date_sk = d_date_sk AND ws_addr_sk = ca_address_sk GROUP BY ca_county, d_qoy, d_year) SELECT ss.ca_county, ss.store_sales, ws.web_sales FROM ss, ws WHERE ss.ca_county = ws.ca_county ORDER BY ss.ca_county";
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("supported grouped CTE query");

        assert_eq!(plan.sources().len(), 2);
        assert_eq!(plan.sources()[0].name(), "ss");
        assert_eq!(
            plan.sources()[0]
                .columns()
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            ["ca_county", "d_qoy", "d_year", "store_sales"]
        );
        assert_eq!(
            plan.sources()[0].columns()[3].data_type,
            "Nullable(Float64)"
        );
        assert_eq!(plan.outer().left, "ss");
        assert_eq!(plan.outer().right, "ws");
    }

    #[test]
    fn parses_grouped_case_sum_cte_and_binds_metric_schema() {
        let sql = "WITH wss AS (SELECT d_week_seq, ss_store_sk, SUM(CASE WHEN d_day_name = 'Sunday' THEN ss_sales_price ELSE NULL END) AS sun_sales, SUM(CASE WHEN d_day_name = 'Monday' THEN ss_sales_price ELSE NULL END) AS mon_sales FROM store_sales, date_dim WHERE ss_sold_date_sk = d_date_sk GROUP BY d_week_seq, ss_store_sk) SELECT wss.d_week_seq, wss.sun_sales, s.s_store_name FROM wss, store s WHERE wss.ss_store_sk = s.s_store_sk";
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("grouped CASE CTE query");

        assert_eq!(plan.sources().len(), 1);
        assert_eq!(
            plan.sources()[0]
                .columns()
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            ["d_week_seq", "ss_store_sk", "sun_sales", "mon_sales"]
        );
        assert_eq!(
            plan.sources()[0].columns()[2].data_type,
            "Nullable(Float64)"
        );
        assert_eq!(
            plan.sources()[0].columns()[3].data_type,
            "Nullable(Float64)"
        );
    }

    #[test]
    fn materializes_union_all_cte_before_grouped_cte() {
        let sql = "WITH combined AS (SELECT ws_sold_date_sk AS sold_date_sk, ws_ext_sales_price AS amount FROM web_sales UNION ALL SELECT cs_sold_date_sk AS sold_date_sk, cs_sales_price AS amount FROM catalog_sales), weekly AS (SELECT d_week_seq, SUM(combined.amount) AS total FROM combined, date_dim WHERE combined.sold_date_sk = d_date_sk GROUP BY d_week_seq) SELECT weekly.d_week_seq, weekly.total FROM weekly, date_dim WHERE weekly.d_week_seq = date_dim.d_week_seq LIMIT 5";
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("union CTE followed by grouped CTE");

        assert_eq!(plan.sources().len(), 2);
        let CteSourceQuery::Union(union) = plan.sources()[0].query() else {
            panic!("first CTE is a materialized UNION ALL");
        };
        assert_eq!(union.branches().len(), 2);
        assert_eq!(
            plan.sources()[0]
                .columns()
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            ["sold_date_sk", "amount"]
        );
        assert_eq!(
            plan.sources()[0].columns()[1].data_type,
            "Nullable(Float64)"
        );
        assert!(matches!(plan.sources()[1].query(), CteSourceQuery::Join(_)));
    }

    #[test]
    fn materializes_union_cte_used_by_grouped_cte_and_two_offset_inputs() {
        let sql = "WITH combined AS (SELECT ws_sold_date_sk AS sold_date_sk, ws_ext_sales_price AS sales_price FROM web_sales UNION ALL SELECT cs_sold_date_sk AS sold_date_sk, cs_ext_sales_price AS sales_price FROM catalog_sales), weekly AS (SELECT d_week_seq, SUM(CASE WHEN d_day_name = 'Sunday' THEN sales_price ELSE NULL END) AS sunday FROM combined, date_dim WHERE sold_date_sk = d_date_sk GROUP BY d_week_seq) SELECT first_week.d_week_seq, first_week.sunday / next_week.sunday AS ratio FROM (SELECT weekly.d_week_seq, weekly.sunday FROM weekly, date_dim WHERE date_dim.d_week_seq = weekly.d_week_seq AND d_year = 2001) first_week, (SELECT weekly.d_week_seq, weekly.sunday FROM weekly, date_dim WHERE date_dim.d_week_seq = weekly.d_week_seq AND d_year = 2002) next_week WHERE first_week.d_week_seq = next_week.d_week_seq - 53 ORDER BY first_week.d_week_seq";
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("offset-key derived CTE query");

        assert_eq!(plan.sources().len(), 4);
        assert_eq!(plan.sources()[0].name(), "combined");
        assert_eq!(plan.sources()[1].name(), "weekly");
        assert_eq!(plan.sources()[2].name(), "first_week");
        assert_eq!(plan.sources()[3].name(), "next_week");
        assert_eq!(plan.outer().kind, crate::JoinType::Inner);
        assert!(plan.outer().keys.is_empty());
    }

    #[test]
    fn materializes_grouped_union_cte_with_trailing_text_literal() {
        let sql = "WITH yearly AS (SELECT d_year AS year_, SUM(ws_ext_sales_price) AS sales, 'w' AS channel FROM web_sales, date_dim WHERE ws_sold_date_sk = d_date_sk GROUP BY d_year UNION ALL SELECT d_year AS year_, SUM(cs_ext_sales_price) AS sales, 'c' AS channel FROM catalog_sales, date_dim WHERE cs_sold_date_sk = d_date_sk GROUP BY d_year) SELECT yearly.year_, yearly.sales, yearly.channel FROM yearly, date_dim WHERE yearly.year_ = date_dim.d_year ORDER BY 1 NULLS FIRST LIMIT 5";
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("grouped UNION ALL CTE with a trailing label");

        let CteSourceQuery::Union(union) = plan.sources()[0].query() else {
            panic!("grouped CTE is a materialized union");
        };
        assert_eq!(union.branches().len(), 2);
        assert_eq!(plan.sources()[0].columns().len(), 3);
        assert_eq!(plan.sources()[0].columns()[2].name, "channel");
        assert_eq!(plan.sources()[0].columns()[2].data_type, "Nullable(String)");
        assert!(matches!(
            union.branches()[0],
            crate::DerivedUnionSelectBranch::JoinWithTextLiterals { .. }
        ));
        assert_eq!(plan.outer().order_by[0].0.name, "year_");
    }

    #[test]
    fn rejects_union_cte_that_projects_only_part_of_union_schema() {
        let sql = "WITH combined AS (SELECT sold_date_sk FROM (SELECT ws_sold_date_sk AS sold_date_sk, ws_ext_sales_price AS amount FROM web_sales UNION ALL SELECT cs_sold_date_sk AS sold_date_sk, cs_sales_price AS amount FROM catalog_sales) u) SELECT combined.sold_date_sk, date_dim.d_week_seq FROM combined, date_dim WHERE combined.sold_date_sk = date_dim.d_date_sk LIMIT 5";
        assert!(
            parse_cte_multi_join_with_schema(sql, schema)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn materializes_two_derived_selects_that_read_a_grouped_cte() {
        let sql = "WITH weekly AS (SELECT d_week_seq, ss_store_sk, SUM(CASE WHEN (d_day_name='Sunday') THEN ss_sales_price ELSE NULL END) AS sun_sales, SUM(CASE WHEN (d_day_name='Monday') THEN ss_sales_price ELSE NULL END) AS mon_sales, SUM(CASE WHEN (d_day_name='Tuesday') THEN ss_sales_price ELSE NULL END) AS tue_sales, SUM(CASE WHEN (d_day_name='Wednesday') THEN ss_sales_price ELSE NULL END) AS wed_sales, SUM(CASE WHEN (d_day_name='Thursday') THEN ss_sales_price ELSE NULL END) AS thu_sales, SUM(CASE WHEN (d_day_name='Friday') THEN ss_sales_price ELSE NULL END) AS fri_sales, SUM(CASE WHEN (d_day_name='Saturday') THEN ss_sales_price ELSE NULL END) AS sat_sales FROM store_sales, date_dim WHERE ss_sold_date_sk = d_date_sk GROUP BY d_week_seq, ss_store_sk) SELECT s_store_name1, s_store_id1, d_week_seq1, sun_sales1 / sun_sales2 AS ratio FROM (SELECT s_store_name AS s_store_name1, wss.d_week_seq AS d_week_seq1, s_store_id AS s_store_id1, sun_sales AS sun_sales1 FROM weekly wss, store, date_dim d WHERE d.d_week_seq = wss.d_week_seq AND ss_store_sk = s_store_sk AND d_month_seq BETWEEN 1212 AND 1212 + 11) y, (SELECT s_store_name AS s_store_name2, wss.d_week_seq AS d_week_seq2, s_store_id AS s_store_id2, sun_sales AS sun_sales2 FROM weekly wss, store, date_dim d WHERE d.d_week_seq = wss.d_week_seq AND ss_store_sk = s_store_sk AND d_month_seq BETWEEN 1212 + 12 AND 1212 + 23) x WHERE s_store_id1 = s_store_id2 AND d_week_seq1 = d_week_seq2 - 52 ORDER BY s_store_name1 NULLS FIRST, s_store_id1 NULLS FIRST, d_week_seq1 NULLS FIRST LIMIT 100";
        let (definitions, outer_sql) = split_ctes(sql).unwrap().unwrap();
        let grouped = parse_join_with_schema(&definitions[0].1, |table| {
            schema(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
        })
        .unwrap()
        .expect("weekly CTE query parses");
        let weekly_columns = output_schema(&grouped, schema)
            .unwrap()
            .expect("weekly CTE schema binds");
        let (derived_sources, _) =
            split_two_derived_sources(&tokenize(&outer_sql).unwrap()).expect("two branches");
        for (name, body) in &derived_sources {
            let parsed = parse_join_with_schema(body, |table| {
                if table.eq_ignore_ascii_case("weekly") {
                    Ok(weekly_columns
                        .iter()
                        .map(|column| column.name.clone())
                        .collect())
                } else {
                    schema(table)
                        .map(|columns| columns.into_iter().map(|column| column.name).collect())
                }
            })
            .unwrap()
            .unwrap_or_else(|| panic!("derived source {name} parses"));
            output_schema(&parsed, |table| {
                if table.eq_ignore_ascii_case("weekly") {
                    Ok(weekly_columns.clone())
                } else {
                    schema(table)
                }
            })
            .unwrap()
            .unwrap_or_else(|| panic!("derived source {name} has output schema"));
        }
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("two derived CTE inputs");

        assert_eq!(plan.sources().len(), 3);
        assert_eq!(plan.sources()[0].name(), "weekly");
        assert_eq!(plan.sources()[1].name(), "y");
        assert_eq!(plan.sources()[2].name(), "x");
        assert_eq!(plan.outer().left, "y");
        assert_eq!(plan.outer().right, "x");
        assert_eq!(plan.outer().scalar_projections.len(), 1);
    }

    #[test]
    fn parses_left_anti_join_cte_with_multiple_grouped_sums() {
        let sql = "WITH ws AS (SELECT d_year AS ws_sold_year, ws_item_sk, ws_bill_customer_sk AS ws_customer_sk, SUM(ws_quantity) AS ws_qty, SUM(ws_wholesale_cost) AS ws_wc, SUM(ws_ext_sales_price) AS ws_sp FROM web_sales LEFT JOIN web_returns ON wr_order_number = ws_order_number AND wr_item_sk = ws_item_sk JOIN date_dim ON ws_sold_date_sk = d_date_sk WHERE wr_order_number IS NULL GROUP BY d_year, ws_item_sk, ws_bill_customer_sk) SELECT current_ws.ws_sold_year, current_ws.ws_item_sk, current_ws.ws_qty FROM ws current_ws, ws previous_ws WHERE current_ws.ws_sold_year = previous_ws.ws_sold_year";
        let (definitions, _outer_sql) = split_ctes(sql).unwrap().unwrap();
        let body_result = parse_join_with_schema(&definitions[0].1, |table| {
            schema(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
        })
        .unwrap();
        assert!(body_result.is_some(), "CTE body parse: {body_result:?}");
        let body = body_result.unwrap();
        let schema_result = output_schema(&body, schema).unwrap();
        assert!(
            schema_result.is_some(),
            "CTE output schema: {schema_result:?}"
        );
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("left anti-join CTE query");

        assert_eq!(plan.sources().len(), 1);
        assert_eq!(
            plan.sources()[0]
                .columns()
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            [
                "ws_sold_year",
                "ws_item_sk",
                "ws_customer_sk",
                "ws_qty",
                "ws_wc",
                "ws_sp"
            ]
        );
        assert_eq!(plan.sources()[0].columns()[3].data_type, "Nullable(Int128)");
        assert_eq!(
            plan.sources()[0].columns()[4].data_type,
            "Nullable(Float64)"
        );
        let CteSourceQuery::Join(query) = plan.sources()[0].query() else {
            panic!("CTE uses a joined source");
        };
        assert_eq!(query.kind, crate::JoinType::Left);
        assert_eq!(query.steps.len(), 1);
        assert_eq!(query.steps[0].kind, crate::JoinType::Inner);
    }

    #[test]
    fn parses_coalesce_or_filter_after_a_left_join_chain() {
        let sql = "SELECT ss.ss_customer_sk, ROUND(ss.ss_quantity / 3, 2) AS ratio FROM store_sales ss LEFT JOIN web_sales ws ON ws.ws_item_sk = ss.ss_item_sk LEFT JOIN catalog_sales cs ON cs.cs_item_sk = ss.ss_item_sk WHERE (COALESCE(ws.ws_quantity, 0) > 0 OR COALESCE(cs.cs_quantity, 0) > 0) AND ss.ss_customer_sk = 2000 ORDER BY ratio LIMIT 100";
        let query = parse_join_with_schema(sql, |table| {
            schema(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
        })
        .unwrap()
        .expect("LEFT JOIN chain with a parenthesized COALESCE filter");

        assert_eq!(query.scalar_where.as_ref().unwrap().len(), 2);
        assert_eq!(query.scalar_projections.len(), 1);
        assert_eq!(query.scalar_projections[0].alias, "ratio");
        assert_eq!(query.order_by[0].0.scalar_order_index(), Some(0));
    }

    #[test]
    fn resolves_a_later_cte_from_an_earlier_cte() {
        let sql = "WITH ss AS (SELECT ca_county, d_qoy, d_year, SUM(ss_ext_sales_price) AS store_sales FROM store_sales, date_dim, customer_address WHERE ss_sold_date_sk = d_date_sk AND ss_addr_sk = ca_address_sk GROUP BY ca_county, d_qoy, d_year), ss_copy AS (SELECT ss.ca_county, ss.store_sales FROM ss, date_dim WHERE ss.d_year = d_date_sk) SELECT ss.ca_county, ss_copy.store_sales FROM ss, ss_copy WHERE ss.ca_county = ss_copy.ca_county";
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("dependent CTE query");

        assert_eq!(plan.sources().len(), 2);
        assert_eq!(plan.sources()[1].name(), "ss_copy");
        assert_eq!(plan.sources()[1].columns()[1].name, "store_sales");
    }

    #[test]
    fn parses_grouped_ctes_with_case_comparison_in_the_outer_query() {
        let sql = "WITH ss AS (SELECT ca_county, d_qoy, d_year, SUM(ss_ext_sales_price) AS store_sales FROM store_sales, date_dim, customer_address WHERE ss_sold_date_sk = d_date_sk AND ss_addr_sk = ca_address_sk GROUP BY ca_county, d_qoy, d_year), ws AS (SELECT ca_county, d_qoy, d_year, SUM(ws_ext_sales_price) AS web_sales FROM web_sales, date_dim, customer_address WHERE ws_sold_date_sk = d_date_sk AND ws_addr_sk = ca_address_sk GROUP BY ca_county, d_qoy, d_year) SELECT ss1.ca_county, ss1.d_year, (ws2.web_sales * 1.0) / ws1.web_sales AS web_growth FROM ss ss1, ss ss2, ws ws1, ws ws2 WHERE ss1.ca_county = ss2.ca_county AND ws1.ca_county = ws2.ca_county AND ss1.ca_county = ws1.ca_county AND CASE WHEN ws1.web_sales > 0 THEN (ws2.web_sales * 1.0) / ws1.web_sales ELSE NULL END > CASE WHEN ss1.store_sales > 0 THEN (ss2.store_sales * 1.0) / ss1.store_sales ELSE NULL END ORDER BY ss1.ca_county";
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("supported grouped CTE query");

        assert_eq!(plan.sources().len(), 2);
        assert_eq!(plan.outer().scalar_projections.len(), 1);
        assert_eq!(plan.outer().scalar_projections[0].alias, "web_growth");
        assert_eq!(
            plan.outer().projection_order,
            [
                crate::JoinProjectionOrder::Column(0),
                crate::JoinProjectionOrder::Column(1),
                crate::JoinProjectionOrder::Scalar(0),
            ]
        );
        assert!(plan.outer().scalar_where.is_some());
    }

    #[test]
    fn does_not_materialize_an_unused_union_cte() {
        let sql = "WITH combined AS (SELECT ca_county FROM customer_address UNION ALL SELECT ca_county FROM customer_address) SELECT a.ca_county, b.ca_county FROM customer_address a, customer_address b WHERE a.ca_county = b.ca_county";
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("outer join remains supported");
        assert!(plan.sources().is_empty());
    }

    #[test]
    fn infers_key_only_cte_columns_from_a_two_table_join() {
        let sql = "WITH ssci AS (SELECT ss_customer_sk AS customer_sk, ss_item_sk AS item_sk FROM store_sales, date_dim WHERE ss_sold_date_sk = d_date_sk AND d_month_seq BETWEEN 1200 AND 1211 GROUP BY ss_customer_sk, ss_item_sk) SELECT ssci.customer_sk, ssci.item_sk FROM ssci, ssci other WHERE ssci.customer_sk = other.customer_sk ORDER BY ssci.customer_sk LIMIT 100";
        let plan = parse_cte_multi_join_with_schema(sql, schema)
            .unwrap()
            .expect("key-only CTE query");

        assert_eq!(plan.sources().len(), 1);
        assert_eq!(
            plan.sources()[0]
                .columns()
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            ["customer_sk", "item_sk"]
        );
        assert!(matches!(
            plan.sources()[0].query(),
            CteSourceQuery::Join(query) if query.grouped_keys_only
        ));
    }
}
