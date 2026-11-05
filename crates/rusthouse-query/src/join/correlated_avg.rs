//! A bounded correlated AVG predicate over an inner-join row.
use super::{JoinColumn, JoinSelect, ParseError, Parser, Token, alias, projected, tokenize};
use crate::Operator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinCorrelatedAvg {
    pub(crate) outer: JoinSelect,
    pub table: String,
    pub inner_key: String,
    pub inner_value: String,
    pub(crate) inner_join: Option<Box<JoinSelect>>,
    pub(crate) inner_tables: Vec<String>,
    pub outer_key: JoinColumn,
    pub outer_value: JoinColumn,
    pub factor: String,
    pub operator: Operator,
}

impl JoinCorrelatedAvg {
    pub fn outer(&self) -> &JoinSelect {
        &self.outer
    }
    pub fn inner_tables(&self) -> &[String] {
        &self.inner_tables
    }
    pub fn has_joined_inner(&self) -> bool {
        self.inner_join.is_some()
    }
    pub fn cap_rows(&mut self, cap: usize) {
        self.outer.limit = Some(self.outer.limit.map_or(cap, |limit| limit.min(cap)));
    }
}

fn keyword(token: &Token, name: &str) -> bool {
    matches!(token, Token::Word(word) if word.eq_ignore_ascii_case(name))
}

fn split_conjuncts(tokens: &[Token]) -> Result<Vec<&[Token]>, ParseError> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut between_and = false;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| ParseError("unbalanced JOIN WHERE".into()))?;
            }
            token if depth == 0 && keyword(token, "BETWEEN") => between_and = true,
            token if depth == 0 && keyword(token, "AND") && between_and => {
                between_and = false;
            }
            token if depth == 0 && keyword(token, "AND") => {
                pieces.push(&tokens[start..index]);
                start = index + 1;
            }
            token if depth == 0 && keyword(token, "OR") => {
                return Err(ParseError(
                    "correlated AVG requires conjunctive filters".into(),
                ));
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(ParseError("unbalanced JOIN WHERE".into()));
    }
    pieces.push(&tokens[start..]);
    if pieces.iter().any(|piece| piece.is_empty()) {
        return Err(ParseError("incomplete JOIN WHERE".into()));
    }
    Ok(pieces)
}

struct InnerSource {
    table: String,
    alias: String,
    columns: Vec<String>,
}

fn parse_inner_sources(
    tokens: &[Token],
    resolve: &mut dyn FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Vec<InnerSource>, ParseError> {
    let mut parser = Parser {
        tokens: tokens.to_vec(),
        index: 0,
    };
    let mut sources = Vec::new();
    loop {
        let table = parser.bounded_word()?;
        let source_alias = alias(&mut parser, &table)?;
        sources.push(InnerSource {
            table: table.clone(),
            alias: source_alias,
            columns: resolve(&table)?,
        });
        if sources.len() > 8 {
            return Err(ParseError(
                "correlated AVG supports at most 8 inner tables".into(),
            ));
        }
        if !parser.take(Token::Comma) {
            break;
        }
    }
    if parser.index != parser.tokens.len() || sources.is_empty() {
        return Err(ParseError(
            "correlated AVG requires named inner tables".into(),
        ));
    }
    Ok(sources)
}

fn inner_source_index(column: &JoinColumn, sources: &[InnerSource]) -> Option<usize> {
    let mut found = None;
    for (index, source) in sources.iter().enumerate() {
        if (!column.relation.is_empty()
            && column.relation != source.alias
            && column.relation != source.table)
            || !source.columns.contains(&column.name)
        {
            continue;
        }
        if found.replace(index).is_some() {
            return None;
        }
    }
    found
}

fn column_tokens(column: &JoinColumn) -> Vec<Token> {
    let mut tokens = Vec::new();
    if !column.relation.is_empty() {
        tokens.extend([Token::Word(column.relation.clone()), Token::Dot]);
    }
    tokens.push(Token::Word(column.name.clone()));
    tokens
}

fn parse_avg_value(parser: &mut Parser) -> Result<(String, JoinColumn), ParseError> {
    let (factor, leading_factor) = match parser.tokens.get(parser.index) {
        Some(Token::Number(_)) => (parser.number()?, true),
        _ => ("1".into(), false),
    };
    if leading_factor && !parser.take(Token::Star) {
        return Err(ParseError("correlated AVG factor requires '*'".into()));
    }
    if !parser.keyword("AVG") || !parser.take(Token::LParen) {
        return Err(ParseError("correlated AVG requires AVG(column)".into()));
    }
    let value = projected(parser)?;
    if value.name == "*" || !parser.take(Token::RParen) {
        return Err(ParseError("correlated AVG requires one column".into()));
    }
    let factor = if leading_factor {
        factor
    } else if parser.take(Token::Star) {
        parser.number()?
    } else {
        factor
    };
    if !factor.parse::<f64>().is_ok_and(f64::is_finite) {
        return Err(ParseError("correlated AVG factor must be finite".into()));
    }
    Ok((factor, value))
}

fn parse_joined_inner_avg(
    from_tokens: &[Token],
    inner_conditions: &[&[Token]],
    inner_value: &JoinColumn,
    resolve: &mut dyn FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<(JoinSelect, Vec<String>, JoinColumn), ParseError> {
    let sources = parse_inner_sources(from_tokens, resolve)?;
    let mut correlation = None;
    let mut remaining = Vec::new();
    for condition in inner_conditions {
        let mut parser = Parser {
            tokens: condition.to_vec(),
            index: 0,
        };
        let parsed = projected(&mut parser).ok().and_then(|left| {
            parser
                .operator()
                .ok()
                .filter(|operator| *operator == Operator::Eq)
                .and_then(|_| projected(&mut parser).ok().map(|right| (left, right)))
        });
        if let Some((left, right)) = parsed.filter(|_| parser.index == parser.tokens.len()) {
            let left_inner = inner_source_index(&left, &sources);
            let right_inner = inner_source_index(&right, &sources);
            match (left_inner, right_inner) {
                (Some(_), None) | (None, Some(_)) => {
                    if correlation.is_some() {
                        return Err(ParseError(
                            "correlated AVG requires one outer equality key".into(),
                        ));
                    }
                    correlation = Some(if left_inner.is_some() {
                        (left, right)
                    } else {
                        (right, left)
                    });
                    continue;
                }
                (Some(_), Some(_)) => {}
                (None, None) => {
                    return Err(ParseError(
                        "correlated AVG inner equality has no inner column".into(),
                    ));
                }
            }
        }
        remaining.push(*condition);
    }
    let Some((inner_key, outer_key)) = correlation else {
        return Err(ParseError(
            "correlated AVG requires an outer equality key".into(),
        ));
    };

    let mut tokens = vec![Token::Word("SELECT".into())];
    tokens.extend(column_tokens(&inner_key));
    tokens.extend([Token::Comma, Token::Word("AVG".into()), Token::LParen]);
    tokens.extend(column_tokens(inner_value));
    tokens.extend([
        Token::RParen,
        Token::Word("AS".into()),
        Token::Word("avg_value".into()),
        Token::Word("FROM".into()),
    ]);
    tokens.extend_from_slice(from_tokens);
    if !remaining.is_empty() {
        tokens.push(Token::Word("WHERE".into()));
        for (index, condition) in remaining.iter().enumerate() {
            if index > 0 {
                tokens.push(Token::Word("AND".into()));
            }
            tokens.extend_from_slice(condition);
        }
    }
    tokens.push(Token::Word("GROUP".into()));
    tokens.push(Token::Word("BY".into()));
    tokens.extend(column_tokens(&inner_key));
    let rewritten = super::implicit::rewrite(tokens, Some(resolve))?;
    let inner = super::parse_correlated_avg_inner_tokens(rewritten)?
        .ok_or_else(|| ParseError("correlated AVG inner query requires a JOIN".into()))?;
    if inner.kind != super::JoinType::Inner
        || inner
            .steps
            .iter()
            .any(|step| step.kind != super::JoinType::Inner)
        || inner.grouped_avg_metrics.len() != 1
        || inner.grouped_count.is_none()
    {
        return Err(ParseError(
            "correlated AVG inner query requires a grouped INNER AVG".into(),
        ));
    }
    Ok((
        inner,
        sources.into_iter().map(|source| source.table).collect(),
        outer_key,
    ))
}

/// Detect a top-level comparison to a SELECT containing an AVG call.
pub fn has_join_correlated_avg(sql: &str) -> Result<bool, ParseError> {
    let tokens = tokenize(sql)?;
    let mut depth = 0usize;
    let mut where_seen = false;
    for (index, token) in tokens.iter().enumerate() {
        if depth == 0 && keyword(token, "WHERE") {
            where_seen = true;
        }
        if depth == 0
            && where_seen
            && ["GROUP", "ORDER", "LIMIT", "OFFSET"]
                .iter()
                .any(|name| keyword(token, name))
        {
            break;
        }
        if depth == 0
            && where_seen
            && matches!(
                token,
                Token::Operator(
                    Operator::Eq
                        | Operator::Ne
                        | Operator::Lt
                        | Operator::Le
                        | Operator::Gt
                        | Operator::Ge
                )
            )
            && tokens.get(index + 1) == Some(&Token::LParen)
            && tokens
                .get(index + 2)
                .is_some_and(|next| keyword(next, "SELECT"))
        {
            let mut inner_depth = 1usize;
            for i in index + 2..tokens.len() {
                if inner_depth == 1
                    && keyword(&tokens[i], "AVG")
                    && tokens.get(i + 1) == Some(&Token::LParen)
                {
                    return Ok(true);
                }
                match tokens[i] {
                    Token::LParen => inner_depth += 1,
                    Token::RParen => {
                        inner_depth -= 1;
                        if inner_depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(false)
}

/// Parse one conjunctive comparison to an uncorrelated-per-key AVG lookup.
pub fn parse_join_correlated_avg_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<JoinCorrelatedAvg>, ParseError> {
    let tokens = tokenize(sql)?;
    let mut depth = 0usize;
    let mut where_index = None;
    let mut end = tokens.len();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ if depth == 0 && where_index.is_none() && keyword(token, "WHERE") => {
                where_index = Some(index)
            }
            Token::Semicolon if depth == 0 && where_index.is_some() => {
                end = index;
                break;
            }
            _ if depth == 0
                && where_index.is_some()
                && ["GROUP", "ORDER", "LIMIT", "OFFSET"]
                    .iter()
                    .any(|name| keyword(token, name)) =>
            {
                end = index;
                break;
            }
            _ => {}
        }
    }
    let Some(where_index) = where_index else {
        return Ok(None);
    };
    let mut pieces = Vec::new();
    let mut start = where_index + 1;
    depth = 0;
    let mut between_and = false;
    for index in where_index + 1..end {
        match &tokens[index] {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| ParseError("unbalanced JOIN WHERE".into()))?
            }
            token if depth == 0 && keyword(token, "OR") => {
                return Ok(None);
            }
            token if depth == 0 && keyword(token, "BETWEEN") => between_and = true,
            token if depth == 0 && keyword(token, "AND") && between_and => {
                between_and = false;
            }
            token if depth == 0 && keyword(token, "AND") => {
                pieces.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(ParseError("unbalanced JOIN WHERE".into()));
    }
    pieces.push(&tokens[start..end]);
    let mut found = None;
    let mut remaining = Vec::new();
    for piece in pieces {
        if piece.is_empty() {
            return Err(ParseError("incomplete JOIN WHERE".into()));
        }
        let candidate = piece.iter().any(|token| keyword(token, "SELECT"))
            && piece
                .windows(2)
                .any(|pair| keyword(&pair[0], "AVG") && pair[1] == Token::LParen);
        if !candidate {
            remaining.push(piece);
            continue;
        }
        if found.is_some() {
            return Err(ParseError("JOIN supports one correlated AVG".into()));
        }
        let mut parser = Parser {
            tokens: piece.to_vec(),
            index: 0,
        };
        let outer_value = projected(&mut parser)?;
        let operator = parser.operator()?;
        if !matches!(
            operator,
            Operator::Eq | Operator::Ne | Operator::Lt | Operator::Le | Operator::Gt | Operator::Ge
        ) || !parser.take(Token::LParen)
            || !parser.keyword("SELECT")
        {
            return Err(ParseError(
                "correlated AVG requires a comparison to (SELECT ...)".into(),
            ));
        }
        let (factor, inner_value) = parse_avg_value(&mut parser)?;
        if !parser.keyword("FROM") {
            return Err(ParseError("correlated AVG requires a table".into()));
        }
        let from_start = parser.index;
        let mut source_depth = 0usize;
        let mut where_source = None;
        for index in from_start..parser.tokens.len() {
            match &parser.tokens[index] {
                Token::LParen => source_depth += 1,
                Token::RParen if source_depth > 0 => source_depth -= 1,
                token if source_depth == 0 && keyword(token, "WHERE") => {
                    where_source = Some(index);
                    break;
                }
                _ => {}
            }
        }
        let where_source = where_source
            .ok_or_else(|| ParseError("correlated AVG requires an inner WHERE".into()))?;
        let from_tokens = parser.tokens[from_start..where_source].to_vec();
        parser.index = where_source + 1;
        let where_start = parser.index;
        let mut condition_depth = 0usize;
        while parser.index < parser.tokens.len() {
            match parser.tokens[parser.index] {
                Token::LParen => condition_depth += 1,
                Token::RParen if condition_depth == 0 => break,
                Token::RParen => condition_depth -= 1,
                _ => {}
            }
            parser.index += 1;
        }
        let inner_conditions = split_conjuncts(&parser.tokens[where_start..parser.index])?;
        let sources = parse_inner_sources(&from_tokens, &mut resolve)?;
        let (table, inner_key, outer_key, inner_join, inner_tables) = if sources.len() > 1 {
            let (inner, tables, outer_key) = parse_joined_inner_avg(
                &from_tokens,
                &inner_conditions,
                &inner_value,
                &mut resolve,
            )?;
            let inner_key = inner
                .grouped_count
                .as_ref()
                .and_then(|columns| columns.first())
                .ok_or_else(|| ParseError("correlated AVG requires an inner group key".into()))?
                .name
                .clone();
            (
                sources[0].table.clone(),
                inner_key,
                outer_key,
                Some(Box::new(inner)),
                tables,
            )
        } else {
            let source = &sources[0];
            let mut correlation = None;
            for condition in &inner_conditions {
                let mut condition_parser = Parser {
                    tokens: condition.to_vec(),
                    index: 0,
                };
                let left = projected(&mut condition_parser)?;
                if condition_parser.operator()? != Operator::Eq {
                    return Err(ParseError("correlated AVG requires an equality key".into()));
                }
                let right = projected(&mut condition_parser)?;
                if condition_parser.index != condition_parser.tokens.len() {
                    return Err(ParseError(
                        "single-table correlated AVG supports one equality filter".into(),
                    ));
                }
                let left_inner = inner_source_index(&left, std::slice::from_ref(source));
                let right_inner = inner_source_index(&right, std::slice::from_ref(source));
                match (left_inner, right_inner) {
                    (Some(_), None) | (None, Some(_)) if correlation.is_none() => {
                        correlation = Some(if left_inner.is_some() {
                            (left.name, right)
                        } else {
                            (right.name, left)
                        });
                    }
                    _ => {
                        return Err(ParseError(
                            "correlated AVG requires one unambiguous outer equality key".into(),
                        ));
                    }
                }
            }
            let Some((inner_key, outer_key)) = correlation else {
                return Err(ParseError("correlated AVG requires an equality key".into()));
            };
            (
                source.table.clone(),
                inner_key,
                outer_key,
                None,
                vec![source.table.clone()],
            )
        };
        if !parser.take(Token::RParen) || parser.index != parser.tokens.len() {
            return Err(ParseError("unsupported correlated AVG syntax".into()));
        }
        if !sources
            .iter()
            .any(|source| inner_source_index(&inner_value, std::slice::from_ref(source)).is_some())
        {
            return Err(ParseError("unknown correlated AVG column".into()));
        }
        found = Some((
            table,
            inner_key,
            inner_value,
            outer_key,
            outer_value,
            factor,
            operator,
            inner_join,
            inner_tables,
        ));
    }
    let Some((
        table,
        inner_key,
        inner_value,
        outer_key,
        outer_value,
        factor,
        operator,
        inner_join,
        inner_tables,
    )) = found
    else {
        return Ok(None);
    };
    let mut outer = tokens[..where_index].to_vec();
    if !remaining.is_empty() {
        outer.push(Token::Word("WHERE".into()));
        for (index, piece) in remaining.iter().enumerate() {
            if index != 0 {
                outer.push(Token::Word("AND".into()));
            }
            outer.extend_from_slice(piece);
        }
    }
    outer.extend_from_slice(&tokens[end..]);
    let rewritten = super::implicit::rewrite(outer, Some(&mut resolve))?;
    let outer = super::parse_join_tokens(rewritten, false)?
        .ok_or_else(|| ParseError("correlated AVG requires an outer JOIN".into()))?;
    if outer.kind != super::JoinType::Inner
        || outer
            .steps
            .iter()
            .any(|step| step.kind != super::JoinType::Inner)
        || (outer.scalar_formula.is_none() && outer.scalar_sum.is_none() && outer.columns.is_none())
    {
        return Err(ParseError(
            "correlated AVG requires a scalar INNER JOIN aggregate formula".into(),
        ));
    }
    Ok(Some(JoinCorrelatedAvg {
        outer,
        table,
        inner_key,
        inner_value: inner_value.name,
        inner_join,
        inner_tables,
        outer_key,
        outer_value,
        factor,
        operator,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn avg_column_name_does_not_trigger_avg_function_parser() {
        let sql = "SELECT SUM(a * 1) AS total FROM lefts, rights WHERE left_key = right_key AND a IN (SELECT avg FROM other)";
        assert!(
            parse_join_correlated_avg_with_schema(sql, |_| panic!("schema is unnecessary"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn detects_avg_subquery_without_matching_literals_or_column_names() {
        let query = include_str!("../../../../bench/tpch/queries/q17.sql");
        assert!(has_join_correlated_avg(query).unwrap());
        assert!(
            !has_join_correlated_avg(
                "SELECT a.x FROM a JOIN b ON a.id = b.id WHERE a.x = 'AVG(1)' -- AVG(x)\n"
            )
            .unwrap()
        );
        assert!(
            !has_join_correlated_avg(
                "SELECT a.x FROM a JOIN b ON a.id = b.id WHERE a.x < (SELECT avg FROM t)"
            )
            .unwrap()
        );
        assert!(!has_join_correlated_avg("SELECT AVG(a) FROM t WHERE a = 1").unwrap());
    }

    #[test]
    fn parses_tpcds_style_joined_avg_with_between_and_greater_than() {
        let sql = "SELECT SUM(cs_ext_discount_amt) AS total FROM catalog_sales, item, date_dim WHERE i_manufact_id = 977 AND i_item_sk = cs_item_sk AND d_date BETWEEN '2000-01-27' AND CAST('2000-04-26' AS DATE) AND d_date_sk = cs_sold_date_sk AND cs_ext_discount_amt > (SELECT 1.3 * AVG(cs_ext_discount_amt) FROM catalog_sales, date_dim WHERE cs_item_sk = i_item_sk AND d_date BETWEEN '2000-01-27' AND CAST('2000-04-26' AS DATE) AND d_date_sk = cs_sold_date_sk) ORDER BY SUM(cs_ext_discount_amt) LIMIT 100";
        let schema = |table: &str| -> Result<Vec<String>, ParseError> {
            let names = match table {
                "catalog_sales" => vec!["cs_item_sk", "cs_sold_date_sk", "cs_ext_discount_amt"],
                "item" => vec!["i_item_sk", "i_manufact_id"],
                "date_dim" => vec!["d_date", "d_date_sk"],
                _ => return Err(ParseError("unknown test table".into())),
            };
            Ok(names.into_iter().map(str::to_owned).collect())
        };
        let plan = parse_join_correlated_avg_with_schema(sql, schema)
            .unwrap()
            .unwrap();

        assert!(plan.inner_join.is_some());
        assert_eq!(plan.inner_tables(), ["catalog_sales", "date_dim"]);
        assert_eq!(plan.operator, Operator::Gt);
        assert_eq!(plan.outer_key.name, "i_item_sk");
        assert!(plan.outer.scalar_sum.is_some());
        assert_eq!(plan.outer.order_by.len(), 1);
        assert!(plan.outer.order_by[0].0.is_count_order());
    }
}
