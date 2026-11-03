//! Bounded grouped substring query with an average filter and anti-join.
use super::{JoinColumn, ParseError, Parser, Token, alias, bound, projected, tokenize};
use crate::Operator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubstringAntiJoin {
    pub table: String,
    pub lookup_table: String,
    pub phone: String,
    pub amount: String,
    pub customer_key: String,
    pub lookup_key: String,
    pub start: usize,
    pub length: usize,
    pub selected_prefixes: Vec<String>,
    pub average_prefixes: Vec<String>,
    pub average_threshold: String,
    pub group_name: String,
    pub count_name: String,
    pub sum_name: String,
    pub descending: bool,
    pub limit: Option<usize>,
    pub offset: usize,
}
impl SubstringAntiJoin {
    pub fn cap_rows(&mut self, cap: usize) {
        self.limit = Some(self.limit.map_or(cap, |limit| limit.min(cap)));
    }
}
fn kw(token: &Token, value: &str) -> bool {
    matches!(token, Token::Word(word) if word.eq_ignore_ascii_case(value))
}
fn required(ok: bool, message: &str) -> Result<(), ParseError> {
    if ok {
        Ok(())
    } else {
        Err(ParseError(message.into()))
    }
}
fn substring(parser: &mut Parser) -> Result<(JoinColumn, usize, usize), ParseError> {
    required(
        parser.keyword("SUBSTRING") && parser.take(Token::LParen),
        "expected SUBSTRING(",
    )?;
    let column = projected(parser)?;
    required(
        column.name != "*" && parser.keyword("FROM"),
        "SUBSTRING requires a column and FROM",
    )?;
    let start = bound(parser, "SUBSTRING start")?;
    required(parser.keyword("FOR"), "SUBSTRING requires FOR")?;
    let length = bound(parser, "SUBSTRING length")?;
    required(
        parser.take(Token::RParen) && (1..=64).contains(&start) && (1..=64).contains(&length),
        "SUBSTRING bounds must be in 1..64",
    )?;
    Ok((column, start, length))
}
fn prefixes(parser: &mut Parser) -> Result<Vec<String>, ParseError> {
    required(
        parser.keyword("IN") && parser.take(Token::LParen),
        "substring filter requires IN",
    )?;
    let mut values = Vec::new();
    loop {
        if values.len() >= 4096 {
            return Err(ParseError(
                "substring IN supports at most 4096 values".into(),
            ));
        }
        let Some(Token::String(value)) = parser.tokens.get(parser.index) else {
            return Err(ParseError("substring IN requires string literals".into()));
        };
        values.push(value.clone());
        parser.index += 1;
        if !parser.take(Token::Comma) {
            break;
        }
    }
    required(parser.take(Token::RParen), "substring IN requires ')'")?;
    Ok(values)
}
fn local(column: &JoinColumn, alias: &str, schema: &[String]) -> Result<(), ParseError> {
    if column.name == "*"
        || (!column.relation.is_empty() && column.relation != alias)
        || !schema.contains(&column.name)
    {
        return Err(ParseError(format!(
            "unknown source column: {}",
            column.name
        )));
    }
    Ok(())
}

/// Detect the supported derived substring/AVG/NOT EXISTS query shape.
pub fn has_substring_antijoin(sql: &str) -> Result<bool, ParseError> {
    let tokens = tokenize(sql)?;
    let mut depth = 0usize;
    let mut derived = false;
    for (index, token) in tokens.iter().enumerate() {
        if depth == 0
            && kw(token, "FROM")
            && tokens.get(index + 1) == Some(&Token::LParen)
            && tokens.get(index + 2).is_some_and(|t| kw(t, "SELECT"))
        {
            derived = true;
        }
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(derived
        && tokens
            .windows(2)
            .any(|p| kw(&p[0], "SUBSTRING") && p[1] == Token::LParen)
        && tokens
            .windows(2)
            .any(|p| kw(&p[0], "AVG") && p[1] == Token::LParen)
        && tokens
            .windows(2)
            .any(|p| kw(&p[0], "NOT") && kw(&p[1], "EXISTS")))
}

/// Parse the supported projection, filters, scalar average, and anti-join.
pub fn parse_substring_antijoin_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<SubstringAntiJoin>, ParseError> {
    if !has_substring_antijoin(sql)? {
        return Ok(None);
    }
    let mut p = Parser {
        tokens: tokenize(sql)?,
        index: 0,
    };
    required(p.keyword("SELECT"), "substring anti-join requires SELECT")?;
    let group_name = p.bounded_word()?;
    required(
        p.take(Token::Comma)
            && p.keyword("COUNT")
            && p.take(Token::LParen)
            && p.take(Token::Star)
            && p.take(Token::RParen)
            && p.keyword("AS"),
        "substring anti-join requires COUNT(*) alias",
    )?;
    let count_name = p.bounded_word()?;
    required(
        p.take(Token::Comma) && p.keyword("SUM") && p.take(Token::LParen),
        "substring anti-join requires SUM(column)",
    )?;
    let outer_amount = projected(&mut p)?;
    required(
        p.take(Token::RParen) && p.keyword("AS"),
        "substring anti-join requires SUM alias",
    )?;
    let sum_name = p.bounded_word()?;
    if group_name.eq_ignore_ascii_case(&count_name)
        || group_name.eq_ignore_ascii_case(&sum_name)
        || count_name.eq_ignore_ascii_case(&sum_name)
        || !outer_amount.relation.is_empty()
    {
        return Err(ParseError("ambiguous derived output".into()));
    }
    required(
        p.keyword("FROM") && p.take(Token::LParen) && p.keyword("SELECT"),
        "substring anti-join requires derived SELECT",
    )?;
    let (phone, start, length) = substring(&mut p)?;
    required(p.keyword("AS"), "substring requires an alias")?;
    let inner_group_name = p.bounded_word()?;
    required(
        inner_group_name.eq_ignore_ascii_case(&group_name) && p.take(Token::Comma),
        "derived group alias differs",
    )?;
    let amount = projected(&mut p)?;
    required(
        !group_name.eq_ignore_ascii_case(&amount.name),
        "ambiguous derived output",
    )?;
    required(
        amount.name == outer_amount.name && p.keyword("FROM"),
        "derived SUM column differs",
    )?;
    let table = p.bounded_word()?;
    let outer_alias = alias(&mut p, &table)?;
    let schema = resolve(&table)?;
    local(&phone, &outer_alias, &schema)?;
    local(&amount, &outer_alias, &schema)?;
    required(p.keyword("WHERE"), "substring anti-join requires WHERE")?;
    let (filter_phone, filter_start, filter_length) = substring(&mut p)?;
    local(&filter_phone, &outer_alias, &schema)?;
    required(
        filter_phone.name == phone.name && filter_start == start && filter_length == length,
        "substring filter differs from projection",
    )?;
    let selected_prefixes = prefixes(&mut p)?;
    required(
        p.keyword("AND"),
        "substring anti-join requires average filter",
    )?;
    let filtered_amount = projected(&mut p)?;
    local(&filtered_amount, &outer_alias, &schema)?;
    required(
        filtered_amount.name == amount.name
            && p.operator()? == Operator::Gt
            && p.take(Token::LParen)
            && p.keyword("SELECT")
            && p.keyword("AVG")
            && p.take(Token::LParen),
        "substring anti-join requires column > (SELECT AVG(...))",
    )?;
    let avg_amount = projected(&mut p)?;
    required(
        p.take(Token::RParen) && p.keyword("FROM"),
        "AVG requires a source",
    )?;
    let avg_table = p.bounded_word()?;
    let avg_alias = alias(&mut p, &avg_table)?;
    required(
        avg_table == table && p.keyword("WHERE"),
        "AVG requires the same source",
    )?;
    local(&avg_amount, &avg_alias, &schema)?;
    required(avg_amount.name == amount.name, "AVG column differs")?;
    let positive_amount = projected(&mut p)?;
    local(&positive_amount, &avg_alias, &schema)?;
    required(
        positive_amount.name == amount.name && p.operator()? == Operator::Gt,
        "AVG requires a numeric threshold",
    )?;
    let average_threshold = p.number()?;
    required(p.keyword("AND"), "AVG requires a substring filter")?;
    let (avg_phone, avg_start, avg_length) = substring(&mut p)?;
    local(&avg_phone, &avg_alias, &schema)?;
    required(
        avg_phone.name == phone.name && avg_start == start && avg_length == length,
        "AVG substring differs",
    )?;
    let average_prefixes = prefixes(&mut p)?;
    required(
        p.take(Token::RParen)
            && p.keyword("AND")
            && p.keyword("NOT")
            && p.keyword("EXISTS")
            && p.take(Token::LParen)
            && p.keyword("SELECT")
            && p.take(Token::Star)
            && p.keyword("FROM"),
        "substring anti-join requires NOT EXISTS",
    )?;
    let lookup_table = p.bounded_word()?;
    let lookup_alias = alias(&mut p, &lookup_table)?;
    if lookup_alias == outer_alias {
        return Err(ParseError("anti-join alias shadows the source".into()));
    }
    let lookup_schema = resolve(&lookup_table)?;
    required(p.keyword("WHERE"), "NOT EXISTS requires WHERE")?;
    let left = projected(&mut p)?;
    required(
        p.operator()? == Operator::Eq,
        "NOT EXISTS requires equality",
    )?;
    let right = projected(&mut p)?;
    let inner = |column: &JoinColumn| -> Result<bool, ParseError> {
        if !column.relation.is_empty() {
            if column.relation != lookup_alias {
                return Ok(false);
            }
            if !lookup_schema.contains(&column.name) {
                return Err(ParseError("unknown NOT EXISTS inner column".into()));
            }
            return Ok(true);
        }
        Ok(lookup_schema.contains(&column.name))
    };
    let (lookup_key, customer_key) = match (inner(&left)?, inner(&right)?) {
        (true, false) => (left, right),
        (false, true) => (right, left),
        _ => return Err(ParseError("NOT EXISTS requires one key per source".into())),
    };
    local(&customer_key, &outer_alias, &schema)?;
    required(
        p.take(Token::RParen) && p.take(Token::RParen),
        "substring anti-join requires closing parentheses",
    )?;
    p.keyword("AS");
    let _derived_alias = p.bounded_word()?;
    required(
        p.keyword("GROUP")
            && p.keyword("BY")
            && p.bounded_word()?.eq_ignore_ascii_case(&group_name),
        "substring anti-join requires GROUP BY alias",
    )?;
    let descending = if p.keyword("ORDER") {
        required(
            p.keyword("BY") && p.bounded_word()?.eq_ignore_ascii_case(&group_name),
            "ORDER BY requires group alias",
        )?;
        if p.keyword("DESC") {
            true
        } else {
            p.keyword("ASC");
            false
        }
    } else {
        false
    };
    let limit = if p.keyword("LIMIT") {
        Some(bound(&mut p, "LIMIT")?)
    } else {
        None
    };
    let offset = if p.keyword("OFFSET") {
        bound(&mut p, "OFFSET")?
    } else {
        0
    };
    p.take(Token::Semicolon);
    required(
        p.index == p.tokens.len(),
        "unsupported substring anti-join clause",
    )?;
    Ok(Some(SubstringAntiJoin {
        table,
        lookup_table,
        phone: phone.name,
        amount: amount.name,
        customer_key: customer_key.name,
        lookup_key: lookup_key.name,
        start,
        length,
        selected_prefixes,
        average_prefixes,
        average_threshold,
        group_name,
        count_name,
        sum_name,
        descending,
        limit,
        offset,
    }))
}
