//! Bounded grouped CTE joined to a table at its maximum metric.
use crate::{AggregateFunction, ParseError, Parser, Projection, Select, Source, Token, tokenize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CteMaxJoin {
    pub(crate) grouped: Select,
    pub(crate) inner_table: String,
    pub(crate) group_column: String,
    pub(crate) group_alias: String,
    pub(crate) metric_alias: String,
    pub(crate) outer_table: String,
    pub(crate) outer_key: String,
    pub(crate) columns: Vec<String>,
    pub(crate) order: Option<(String, bool, Option<bool>)>,
    pub(crate) limit: Option<usize>,
    pub(crate) offset: usize,
}
impl CteMaxJoin {
    pub fn inner_table(&self) -> &str {
        &self.inner_table
    }
    pub fn outer_table(&self) -> &str {
        &self.outer_table
    }
    pub fn cap_rows(&mut self, cap: usize) {
        self.limit = Some(self.limit.map_or(cap, |limit| limit.min(cap)));
    }
}

fn supported_grouped_cte(grouped: &Select) -> bool {
    !grouped.distinct
        && grouped.rename.is_none()
        && grouped.having.is_none()
        && !grouped.hidden_aggregate
        && grouped.limit.is_none()
        && grouped.offset.is_none()
        && grouped.order_by.is_none()
        && grouped.order_by_extra.is_empty()
        && grouped.grouped_expressions.len() == 1
        && grouped.grouped_expressions[0].is_some()
}

fn parse_page_number(parser: &mut Parser) -> Result<usize, ParseError> {
    let value = parser.number()?;
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ParseError("LIMIT or OFFSET requires an integer".into()));
    }
    value
        .parse()
        .map_err(|_| ParseError("LIMIT or OFFSET is too large".into()))
}

fn parse_outer_order(parser: &mut Parser) -> Result<Option<(String, bool, Option<bool>)>, ParseError> {
    if !parser.keyword("ORDER") {
        return Ok(None);
    }
    if !parser.keyword("BY") {
        return Err(ParseError("ORDER requires BY".into()));
    }
    let column = parser.bounded_word()?;
    let descending = if parser.keyword("DESC") {
        true
    } else {
        parser.keyword("ASC");
        false
    };
    let nulls_first = if parser.keyword("NULLS") {
        if parser.keyword("FIRST") {
            Some(true)
        } else if parser.keyword("LAST") {
            Some(false)
        } else {
            return Err(ParseError("NULLS requires FIRST or LAST".into()));
        }
    } else {
        None
    };
    Ok(Some((column, descending, nulls_first)))
}

/// Whether a statement starts with WITH. Unsupported CTE shapes are rejected.
pub fn has_cte_max_join(sql: &str) -> Result<bool, ParseError> {
    Ok(
        matches!(tokenize(sql)?.first(), Some(Token::Word(word)) if word.eq_ignore_ascii_case("WITH")),
    )
}

/// Parse a single grouped SUM CTE joined to a table at the CTE's MAX value.
pub fn parse_cte_max_join(sql: &str) -> Result<Option<CteMaxJoin>, ParseError> {
    let tokens = tokenize(sql)?;
    let mut parser = Parser { tokens, index: 0 };
    if !parser.keyword("WITH") {
        return Ok(None);
    }
    let cte = parser.bounded_word()?;
    if !parser.keyword("AS") || !parser.take(Token::LParen) {
        return Err(ParseError("CTE requires AS (SELECT ...)".into()));
    }
    let start = parser.index;
    let mut depth = 1usize;
    while parser.index < parser.tokens.len() && depth > 0 {
        match parser.tokens[parser.index] {
            Token::LParen => depth += 1,
            Token::RParen => depth -= 1,
            _ => {}
        }
        parser.index += 1;
    }
    if depth != 0 {
        return Err(ParseError("CTE requires closing ')'".into()));
    }
    let mut inner = parser.tokens[start..parser.index - 1].to_vec();
    let mut prefix = Parser {
        tokens: inner.clone(),
        index: 0,
    };
    if !prefix.keyword("SELECT") {
        return Err(ParseError("CTE requires SELECT".into()));
    }
    let group_column = prefix.bounded_word()?;
    if !prefix.keyword("AS") {
        return Err(ParseError("CTE group key requires an alias".into()));
    }
    let group_alias = prefix.bounded_word()?;
    if !prefix.take(Token::Comma) {
        return Err(ParseError("CTE requires a grouped SUM".into()));
    }
    inner.drain(2..4);
    let mut nesting = 0usize;
    let mut group_index = None;
    for (index, token) in inner.iter().enumerate() {
        match token {
            Token::LParen => nesting += 1,
            Token::RParen => nesting = nesting.saturating_sub(1),
            Token::Word(word) if nesting == 0 && word.eq_ignore_ascii_case("GROUP") => {
                group_index = Some(index);
                break;
            }
            _ => {}
        }
    }
    let index = group_index.ok_or_else(|| ParseError("CTE requires GROUP BY".into()))?;
    if !matches!(inner.get(index + 1), Some(Token::Word(word)) if word.eq_ignore_ascii_case("BY"))
        || !matches!(inner.get(index + 2), Some(Token::Word(word) | Token::QuotedWord(word)) if word == &group_alias)
    {
        return Err(ParseError("CTE GROUP BY must use its key alias".into()));
    }
    inner[index + 2] = Token::Word(group_column.clone());
    let grouped = crate::parse_select_tokens(inner)?;
    let metric_alias = match &grouped.projection {
        Projection::GroupedAggregates {
            column,
            items,
            aliases,
            filters,
        } if column == &group_column
            && items.len() == 1
            && items[0].0 == AggregateFunction::Sum
            && aliases.len() == 1
            && filters == &[None] =>
        {
            aliases[0]
                .clone()
                .ok_or_else(|| ParseError("CTE SUM requires an alias".into()))?
        }
        _ => return Err(ParseError("CTE requires one grouped SUM".into())),
    };
    let inner_table = match &grouped.source {
        Some(Source::Table(table)) => table.clone(),
        _ => return Err(ParseError("CTE requires a stored table".into())),
    };
    if !supported_grouped_cte(&grouped) {
        return Err(ParseError("unsupported grouped CTE".into()));
    }
    if group_alias.eq_ignore_ascii_case(&metric_alias) {
        return Err(ParseError("CTE aliases must differ".into()));
    }
    if !parser.keyword("SELECT") {
        return Err(ParseError("CTE requires an outer SELECT".into()));
    }
    let mut columns = Vec::new();
    loop {
        if columns.len() == 64 {
            return Err(ParseError(
                "CTE JOIN supports at most 64 output columns".into(),
            ));
        }
        columns.push(parser.bounded_word()?);
        if !parser.take(Token::Comma) {
            break;
        }
    }
    if !parser.keyword("FROM") {
        return Err(ParseError("CTE JOIN requires FROM".into()));
    }
    let outer_table = parser.bounded_word()?;
    if !parser.take(Token::Comma)
        || parser.bounded_word()? != cte
        || outer_table == cte
        || !parser.keyword("WHERE")
    {
        return Err(ParseError("CTE JOIN requires a table and the CTE".into()));
    }
    let left = parser.bounded_word()?;
    if !parser.take(Token::Operator(crate::Operator::Eq)) {
        return Err(ParseError("CTE JOIN requires an equality key".into()));
    }
    let right = parser.bounded_word()?;
    let outer_key = if left == group_alias && right != group_alias {
        right
    } else if right == group_alias && left != group_alias {
        left
    } else {
        return Err(ParseError("CTE JOIN requires the CTE key".into()));
    };
    if !parser.keyword("AND")
        || parser.bounded_word()? != metric_alias
        || !parser.take(Token::Operator(crate::Operator::Eq))
        || !parser.take(Token::LParen)
        || !parser.keyword("SELECT")
        || !parser.keyword("MAX")
        || !parser.take(Token::LParen)
        || parser.bounded_word()? != metric_alias
        || !parser.take(Token::RParen)
        || !parser.keyword("FROM")
        || parser.bounded_word()? != cte
        || !parser.take(Token::RParen)
    {
        return Err(ParseError(
            "CTE JOIN requires equality to its MAX metric".into(),
        ));
    }
    let order = parse_outer_order(&mut parser)?;
    let limit = if parser.keyword("LIMIT") {
        Some(parse_page_number(&mut parser)?)
    } else {
        None
    };
    let offset = if parser.keyword("OFFSET") {
        parse_page_number(&mut parser)?
    } else {
        0
    };
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError("unsupported CTE JOIN syntax".into()));
    }
    Ok(Some(CteMaxJoin {
        grouped,
        inner_table,
        group_column,
        group_alias,
        metric_alias,
        outer_table,
        outer_key,
        columns,
        order,
        limit,
        offset,
    }))
}
