//! Bounded two-table equi-join syntax for persistent SQL tables.

mod correlated;
mod correlated_avg;
mod correlated_min;
mod correlated_sum;
mod derived_count;
mod derived_group;
mod derived_ratio;
mod derived_sum;
mod exists_lookup;
mod implicit;
mod nested_sum;
mod substring_antijoin;
mod syntax;
mod parse_core;
use parse_core::parse_join_tokens_internal;

// Keep explained execution bounds and the executor's actual limits in sync.
pub(crate) const MAX_BUFFERED_JOIN_ROWS: u64 = 32_768;
pub(crate) const MAX_STREAM_JOIN_BUILD_ROWS: u64 = 262_144;
pub(crate) const MAX_JOIN_RESULT_ROWS: usize = 131_072;
pub(crate) const MAX_JOIN_BUILD_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_JOIN_CHAIN_BYTES: usize = 128 * 1024 * 1024;
const MAX_JOIN_GROUPED_CASE_METRICS: usize = 32;

use self::syntax::*;
pub use correlated::{
    JoinCorrelatedExists, JoinExistsDateFilter, JoinExistsFilter,
    parse_join_correlated_exists_with_schema,
};
pub use correlated_avg::{
    JoinCorrelatedAvg, has_join_correlated_avg, parse_join_correlated_avg_with_schema,
};
pub use correlated_min::{
    JoinCorrelatedMin, has_join_correlated_min, parse_join_correlated_min_with_schema,
};
pub use correlated_sum::{
    JoinCorrelatedSum, has_join_correlated_sum, parse_join_correlated_sum_with_schema,
};
pub use derived_count::{
    DerivedJoinCount, has_derived_join_count, parse_derived_join_count_with_schema,
};
pub(crate) use derived_group::DerivedGroupedFilter;
pub(crate) use derived_group::DerivedGroupedOrderKey;
pub use derived_group::{
    DerivedColumn, DerivedGroupedJoin, DerivedGroupedSelect, DerivedUnionJoin, DerivedUnionSelect,
    DerivedUnionSelectBranch, has_derived_union, parse_derived_grouped_join_with_schema,
    parse_derived_grouped_select_with_schema, parse_derived_union_join_with_schema,
    parse_derived_union_select_with_schema,
};
pub use derived_sum::parse_derived_join_sum_with_schema;
pub(crate) use exists_lookup::JoinExistsLookupPair;
pub use exists_lookup::{JoinExistsKey, JoinExistsLookup};
pub use nested_sum::{NestedJoinSum, has_nested_join_sum, parse_nested_join_sum_with_schema};
pub use substring_antijoin::{
    SubstringAntiJoin, has_substring_antijoin, parse_substring_antijoin_with_schema,
};

use super::{ParseError, Parser, Token, tokenize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinColumn {
    pub relation: String,
    pub name: String,
}

impl JoinColumn {
    pub(crate) fn display_name(&self) -> String {
        if self.relation.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.relation, self.name)
        }
    }

    fn count_order() -> Self {
        Self {
            relation: String::new(),
            name: "#count".into(),
        }
    }

    fn case_order(index: usize) -> Self {
        Self {
            relation: String::new(),
            name: format!("#case:{index}"),
        }
    }

    fn avg_order(index: usize) -> Self {
        Self {
            relation: String::new(),
            name: format!("#avg:{index}"),
        }
    }

    fn grouping_order(index: usize) -> Self {
        Self {
            relation: String::new(),
            name: format!("#grouping:{index}"),
        }
    }

    fn window_ratio_order() -> Self {
        Self {
            relation: String::new(),
            name: "#window_ratio".into(),
        }
    }

    fn scalar_order(index: usize) -> Self {
        Self {
            relation: String::new(),
            name: format!("#scalar:{index}"),
        }
    }

    pub(crate) fn is_count_order(&self) -> bool {
        self.relation.is_empty() && self.name == "#count"
    }

    pub(crate) fn case_order_index(&self) -> Option<usize> {
        self.relation
            .is_empty()
            .then(|| self.name.strip_prefix("#case:")?.parse().ok())
            .flatten()
    }

    pub(crate) fn avg_order_index(&self) -> Option<usize> {
        self.relation
            .is_empty()
            .then(|| self.name.strip_prefix("#avg:")?.parse().ok())
            .flatten()
    }

    pub(crate) fn grouping_order_index(&self) -> Option<usize> {
        self.relation
            .is_empty()
            .then(|| self.name.strip_prefix("#grouping:")?.parse().ok())
            .flatten()
    }

    pub(crate) fn is_window_ratio_order(&self) -> bool {
        self.relation.is_empty() && self.name == "#window_ratio"
    }

    pub(crate) fn scalar_order_index(&self) -> Option<usize> {
        self.relation
            .is_empty()
            .then(|| self.name.strip_prefix("#scalar:")?.parse().ok())
            .flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinVirtualColumn {
    pub relation: String,
    pub name: String,
    pub source: String,
    pub start: usize,
    pub length: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinStep {
    pub table: String,
    pub alias: String,
    pub kind: JoinType,
    pub any: bool,
    pub keys: Vec<(JoinColumn, JoinColumn)>,
    pub using_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinAggregate {
    CountStar,
    CountColumn(JoinColumn),
    Numeric {
        function: crate::AggregateFunction,
        column: JoinColumn,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinScalarAggregate {
    pub aggregate: JoinAggregate,
    pub alias: String,
    pub distinct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinGroupedOutput {
    Key(usize),
    Count(String),
    Sum(String),
    SumExtra(usize, String),
    WindowRatio(String),
    Min(usize, String),
    Max(usize, String),
    Average(usize, String),
    WindowAverage(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedWindowAverage {
    pub value: JoinColumn,
    pub partition: Vec<JoinColumn>,
    pub alias: String,
}

fn join_aggregate_header(aggregate: &JoinAggregate) -> String {
    match aggregate {
        JoinAggregate::CountStar => "count(*)".to_owned(),
        JoinAggregate::CountColumn(column) => {
            format!("count({})", column.display_name())
        }
        JoinAggregate::Numeric { function, column } => {
            format!("{}({})", function.name(), column.display_name())
        }
    }
}

fn join_scalar_aggregate_header(aggregate: &JoinAggregate, distinct: bool) -> String {
    if distinct {
        if let JoinAggregate::CountColumn(column) = aggregate {
            return format!("count(DISTINCT {})", column.display_name());
        }
    }
    join_aggregate_header(aggregate)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupedSum {
    Column(JoinColumn),
    Expression(crate::ScalarExpr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupedCaseValue {
    Count,
    Column(JoinColumn),
    Expression(crate::ScalarExpr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedSumHaving {
    pub operator: crate::Operator,
    pub value: String,
    pub subquery: Option<Box<JoinSelect>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedCountHaving {
    pub operator: crate::Operator,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinAggregateFormula {
    pub sums: Vec<crate::ScalarExpr>,
    pub expression: crate::ScalarExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedWindowRatio {
    pub partition: JoinColumn,
    pub multiplier: String,
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinInSubquery {
    pub column: JoinColumn,
    pub inner: Box<crate::Select>,
    pub negated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinScalarProjection {
    pub expression: crate::ScalarExpr,
    pub alias: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinProjectionOrder {
    Column(usize),
    Scalar(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinSelect {
    pub left: String,
    pub right: String,
    pub left_alias: String,
    pub right_alias: String,
    pub kind: JoinType,
    /// Select one right row per left row for INNER/LEFT, or one left row per right for RIGHT.
    pub any: bool,
    /// None projects every column from both tables.
    pub columns: Option<Vec<JoinColumn>>,
    /// Row-level arithmetic expressions mixed with source-column projections.
    pub scalar_projections: Vec<JoinScalarProjection>,
    /// Output ordering for row-level scalar projections.
    pub projection_order: Vec<JoinProjectionOrder>,
    pub aggregate: Option<JoinAggregate>,
    /// Multiple scalar metrics accumulated while streaming an INNER join chain.
    pub scalar_aggregates: Vec<JoinScalarAggregate>,
    /// User-provided name for a scalar aggregate result column.
    pub aggregate_alias: Option<String>,
    /// Exactly one qualified GROUP BY key with COUNT(*) projected last.
    pub grouped_count: Option<Vec<JoinColumn>>,
    /// Full execution key set, including unprojected GROUP BY keys.
    pub grouped_execution_keys: Vec<JoinColumn>,
    /// Output order for grouped keys and repeated COUNT(*) metrics.
    pub grouped_output: Vec<JoinGroupedOutput>,
    /// Stream distinct projected keys for GROUP BY without an aggregate.
    pub grouped_keys_only: bool,
    /// Count non-NULL values of this column in grouped joins.
    pub grouped_count_column: Option<JoinColumn>,
    /// Right-side LIKE predicate applied before a grouped LEFT JOIN matches.
    pub on_right_like: Option<(JoinColumn, String, bool)>,
    /// Integer COUNT(DISTINCT column) grouped over INNER joins.
    pub grouped_distinct: Option<JoinColumn>,
    /// Conditional SUM metrics for a grouped key; no value column means a CASE count.
    pub grouped_case_counts: Vec<(crate::GroupedFilter, GroupedCaseValue, String)>,
    /// A numeric SUM over an INNER join.
    pub grouped_sum: Option<GroupedSum>,
    /// Additional independent grouped SUM columns on the same join groups.
    pub grouped_sum_extras: Vec<(GroupedSum, String)>,
    /// A numeric MIN over an INNER join.
    pub grouped_min: Option<JoinColumn>,
    /// Additional grouped MIN and MAX metrics in projection order.
    pub grouped_min_metrics: Vec<(JoinColumn, String)>,
    pub grouped_max_metrics: Vec<(JoinColumn, String)>,
    /// Independently nullable numeric AVG metrics over grouped INNER join chains.
    pub grouped_avg_metrics: Vec<(JoinColumn, String)>,
    /// AVG of a grouped SUM over each projected partition.
    pub grouped_window_average: Option<GroupedWindowAverage>,
    /// Add SQL ROLLUP subtotal rows for the projected grouped keys.
    pub grouped_rollup: bool,
    /// Arithmetic over grouped SUM expressions.
    pub grouped_formula: Option<JoinAggregateFormula>,
    /// A grouped SUM divided by the sum of its grouped values per partition.
    pub grouped_window_ratio: Option<GroupedWindowRatio>,
    /// Position and alias of an EXTRACT(YEAR FROM date) grouping key.
    pub grouped_year: Option<(usize, String)>,
    /// Output aliases for projected source columns.
    pub grouped_key_aliases: Vec<(usize, String)>,
    /// Position of the grouped aggregate in the SELECT list.
    pub grouped_sum_position: Option<usize>,
    pub grouped_sum_having: Option<GroupedSumHaving>,
    pub grouped_count_having: Option<GroupedCountHaving>,
    /// A scalar SUM over a numeric expression in an INNER join.
    pub scalar_sum: Option<crate::ScalarExpr>,
    /// Qualified source for a scalar SUM(column) in a HAVING subquery.
    pub scalar_sum_qualifier: Option<String>,
    /// Preserve floating-point type when binding an internal scalar result.
    pub scalar_result_typed: bool,
    /// Arithmetic over scalar SUM results.
    pub scalar_formula: Option<JoinAggregateFormula>,
    /// Bounded OR-of-AND predicates for scalar sums over joins.
    pub scalar_where: Option<Vec<Vec<crate::Predicate>>>,
    /// A bounded single-table IN or NOT IN subquery, bound before execution.
    pub in_subquery: Option<JoinInSubquery>,
    pub keys: Vec<(JoinColumn, JoinColumn)>,
    /// Shared-key names for JOIN USING; star output projects each key once.
    pub using_keys: Vec<String>,
    /// Optional post-join NULL filter (`true` means `IS NOT NULL`).
    pub where_null: Option<(JoinColumn, bool)>,
    /// One typed comparison after a two-table unordered INNER JOIN.
    pub where_comparison: Option<(JoinColumn, crate::Operator, String)>,
    /// Additional conjunctive literal comparisons for two-table INNER joins.
    pub where_comparisons_extra: Vec<(JoinColumn, crate::Operator, String)>,
    /// Column-to-column comparisons for two-table INNER joins.
    pub where_column_comparisons: Vec<(JoinColumn, crate::Operator, JoinColumn)>,
    /// Bounded SUBSTR/SUBSTRING comparisons between JOIN columns.
    pub where_substring_comparisons: Vec<(
        JoinColumn,
        usize,
        usize,
        crate::Operator,
        JoinColumn,
        usize,
        usize,
    )>,
    /// Small IN lists for two-table INNER join filters.
    pub where_in_filters: Vec<(JoinColumn, Vec<String>)>,
    /// LIKE filters: column, pattern, case-insensitive, negated.
    pub where_like_filters: Vec<(JoinColumn, String, bool, bool)>,
    /// Ordered key, descending flag, and optional explicit NULL placement.
    pub order_by: Vec<(JoinColumn, bool, Option<bool>)>,
    pub limit: Option<usize>,
    pub offset: usize,
    /// Additional JOIN steps after the first two-table join.
    pub steps: Vec<JoinStep>,
    /// Bounded SUBSTRING columns projected by simple derived table sources.
    pub virtual_columns: Vec<JoinVirtualColumn>,
}

fn grouped_output_alias_exists(
    output: &[JoinGroupedOutput],
    key_aliases: &[(usize, String)],
    columns: &[JoinColumn],
    alias: &str,
) -> bool {
    output.iter().any(|item| match item {
        JoinGroupedOutput::Count(name)
        | JoinGroupedOutput::Sum(name)
        | JoinGroupedOutput::WindowRatio(name)
        | JoinGroupedOutput::WindowAverage(name)
        | JoinGroupedOutput::Min(_, name)
        | JoinGroupedOutput::Max(_, name)
        | JoinGroupedOutput::Average(_, name) => name.eq_ignore_ascii_case(alias),
        JoinGroupedOutput::SumExtra(_, name) => name.eq_ignore_ascii_case(alias),
        JoinGroupedOutput::Key(index) => columns
            .get(*index)
            .is_some_and(|column| column.name.eq_ignore_ascii_case(alias)),
    }) || key_aliases
        .iter()
        .any(|(_, name)| name.eq_ignore_ascii_case(alias))
}

fn scalar_projection_ahead(parser: &Parser) -> bool {
    if matches!(
        (
            parser.tokens.get(parser.index),
            parser.tokens.get(parser.index + 1),
            parser.tokens.get(parser.index + 2)
        ),
        (
            Some(Token::Word(_) | Token::QuotedWord(_)),
            Some(Token::Dot),
            Some(Token::Star)
        )
    ) {
        return false;
    }
    match parser.tokens.get(parser.index) {
        Some(Token::LParen | Token::Plus | Token::Minus | Token::String(_)) => return true,
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("CASE") => return true,
        Some(Token::Word(word))
            if word.eq_ignore_ascii_case("CONCAT")
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen) =>
        {
            return true;
        }
        Some(Token::Word(word))
            if word.eq_ignore_ascii_case("SUBSTRING")
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen) =>
        {
            return true;
        }
        Some(Token::Word(word))
            if word.eq_ignore_ascii_case("ROUND")
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen) =>
        {
            return true;
        }
        _ => {}
    }

    let mut depth = 0usize;
    for token in &parser.tokens[parser.index..] {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Comma if depth == 0 => break,
            Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("FROM") => break,
            Token::Plus | Token::Minus | Token::Star | Token::Slash | Token::Percent
                if depth == 0 =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

fn implicit_scalar_projection_alias(expression: &crate::ScalarExpr) -> String {
    match expression {
        crate::ScalarExpr::Round { value, precision }
            if let crate::ScalarExpr::Divide(left, right) = value.as_ref() =>
        {
            format!(
                "round(({} / {}), {precision})",
                crate::multi_aggregate::render_expr(left),
                crate::multi_aggregate::render_expr(right)
            )
        }
        crate::ScalarExpr::TextSubstring {
            value,
            start,
            length,
        } => {
            let mut alias = format!(
                "\"substring\"({}, {start}",
                crate::multi_aggregate::render_expr(value)
            );
            if let Some(length) = length {
                alias.push_str(&format!(", {length}"));
            }
            alias.push(')');
            alias
        }
        _ => crate::multi_aggregate::render_expr(expression),
    }
}

pub(crate) fn encode_qualified_expression_tokens(tokens: &[Token]) -> (Vec<Token>, Vec<usize>) {
    let mut encoded = Vec::with_capacity(tokens.len());
    let mut original_ends = Vec::with_capacity(tokens.len());
    let mut index = 0;
    while index < tokens.len() {
        let identifier = |token: &Token| match token {
            Token::Word(value) | Token::QuotedWord(value) => Some(value.clone()),
            _ => None,
        };
        if let (Some(left), Some(Token::Dot), Some(right)) = (
            tokens.get(index).and_then(identifier),
            tokens.get(index + 1),
            tokens.get(index + 2).and_then(identifier),
        ) {
            encoded.push(Token::Word(format!("{left}\0{right}")));
            original_ends.push(index + 3);
            index += 3;
        } else {
            encoded.push(tokens[index].clone());
            original_ends.push(index + 1);
            index += 1;
        }
    }
    (encoded, original_ends)
}

/// Return None for queries without a JOIN keyword after FROM.
pub fn parse_join(sql: &str) -> Result<Option<JoinSelect>, ParseError> {
    parse_join_resolved(tokenize(sql)?, None, false)
}

/// Whether a SELECT or its scalar HAVING subquery has a comma-separated source list.
pub fn has_implicit_join(sql: &str) -> Result<bool, ParseError> {
    let tokens = tokenize(sql)?;
    if implicit::has_comma(&tokens)? {
        return Ok(true);
    }
    let mut depth = 0usize;
    let mut having = false;
    for (index, token) in tokens.iter().enumerate() {
        if depth == 0 {
            if matches!(token, Token::Word(word) if word.eq_ignore_ascii_case("HAVING")) {
                having = true;
            }
            if matches!(token, Token::Word(word) if ["ORDER", "LIMIT", "OFFSET", "FORMAT"].iter().any(|name| word.eq_ignore_ascii_case(name)))
            {
                having = false;
            }
            if having
                && *token == Token::LParen
                && index > 0
                && matches!(tokens[index - 1], Token::Operator(_))
                && matches!(tokens.get(index + 1), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT"))
            {
                let mut nesting = 1usize;
                for end in index + 1..tokens.len() {
                    match tokens[end] {
                        Token::LParen => nesting += 1,
                        Token::RParen => nesting -= 1,
                        _ => {}
                    }
                    if nesting == 0 {
                        return implicit::has_comma(&tokens[index + 1..end]);
                    }
                }
                return Err(ParseError("HAVING subquery requires ')'".into()));
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

/// Resolve unqualified comma-join keys with the schemas held by the caller.
pub fn parse_join_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<JoinSelect>, ParseError> {
    parse_join_resolved(tokenize(sql)?, Some(&mut resolve), false)
}

pub(super) fn parse_derived_outer_join_with_schema(
    sql: &str,
    mut resolve: impl FnMut(&str) -> Result<Vec<String>, ParseError>,
) -> Result<Option<JoinSelect>, ParseError> {
    parse_join_resolved(tokenize(sql)?, Some(&mut resolve), true)
}

/// Whether the outer SELECT has an explicit JOIN.
pub fn has_explicit_join(sql: &str) -> Result<bool, ParseError> {
    let tokens = tokenize(sql)?;
    let mut depth = 0usize;
    let mut from = false;
    for token in &tokens {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("FROM") => from = true,
            Token::Word(word) if depth == 0 && from && word.eq_ignore_ascii_case("JOIN") => {
                return Ok(true);
            }
            Token::Word(word)
                if depth == 0
                    && from
                    && ["WHERE", "GROUP", "ORDER", "LIMIT", "OFFSET"]
                        .iter()
                        .any(|clause| word.eq_ignore_ascii_case(clause)) =>
            {
                break;
            }
            _ => {}
        }
    }
    Ok(false)
}

/// Detect a top-level EXISTS predicate without matching literals or comments.
pub fn has_top_level_exists(sql: &str) -> Result<bool, ParseError> {
    let mut depth = 0usize;
    let mut where_seen = false;
    for token in tokenize(sql)? {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("WHERE") => {
                where_seen = true
            }
            Token::Word(word)
                if depth == 0 && where_seen && word.eq_ignore_ascii_case("EXISTS") =>
            {
                return Ok(true);
            }
            Token::Word(word)
                if depth == 0
                    && where_seen
                    && ["GROUP", "ORDER", "LIMIT", "OFFSET"]
                        .iter()
                        .any(|clause| word.eq_ignore_ascii_case(clause)) =>
            {
                break;
            }
            _ => {}
        }
    }
    Ok(false)
}

type SchemaResolver<'a> = &'a mut dyn FnMut(&str) -> Result<Vec<String>, ParseError>;

fn word_at(tokens: &[Token], index: usize) -> Option<&str> {
    match tokens.get(index)? {
        Token::Word(value) | Token::QuotedWord(value) => Some(value),
        _ => None,
    }
}

fn keyword_at(tokens: &[Token], index: usize, expected: &str) -> bool {
    word_at(tokens, index).is_some_and(|word| word.eq_ignore_ascii_case(expected))
}

fn parse_simple_substring_source(
    tokens: &[Token],
    start: usize,
) -> Option<(usize, String, String, JoinVirtualColumn)> {
    if tokens.get(start) != Some(&Token::LParen)
        || !keyword_at(tokens, start + 1, "SELECT")
        || !keyword_at(tokens, start + 2, "SUBSTRING")
        || tokens.get(start + 3) != Some(&Token::LParen)
    {
        return None;
    }
    let source = word_at(tokens, start + 4)?.to_owned();
    if tokens.get(start + 5) != Some(&Token::Comma) {
        return None;
    }
    let substring_start = match tokens.get(start + 6)? {
        Token::Number(value) => value.parse::<usize>().ok()?,
        _ => return None,
    };
    if tokens.get(start + 7) != Some(&Token::Comma) {
        return None;
    }
    let length = match tokens.get(start + 8)? {
        Token::Number(value) => value.parse::<usize>().ok()?,
        _ => return None,
    };
    if substring_start == 0
        || length == 0
        || length > 4096
        || tokens.get(start + 9) != Some(&Token::RParen)
    {
        return None;
    }
    let mut index = start + 10;
    if keyword_at(tokens, index, "AS") {
        index += 1;
    }
    let name = word_at(tokens, index)?.to_owned();
    if tokens.get(index + 1) != Some(&Token::Comma)
        || tokens.get(index + 2) != Some(&Token::Star)
        || !keyword_at(tokens, index + 3, "FROM")
    {
        return None;
    }
    let table = word_at(tokens, index + 4)?.to_owned();
    if tokens.get(index + 5) != Some(&Token::RParen) {
        return None;
    }
    index += 6;
    if keyword_at(tokens, index, "AS") {
        index += 1;
    }
    let relation = word_at(tokens, index)?.to_owned();
    Some((
        index + 1,
        table,
        relation.clone(),
        JoinVirtualColumn {
            relation,
            name,
            source,
            start: substring_start,
            length,
        },
    ))
}

fn flatten_simple_substring_sources(
    mut tokens: Vec<Token>,
) -> Result<(Vec<Token>, Vec<JoinVirtualColumn>), ParseError> {
    let mut from = None;
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        if depth == 0 && keyword_at(&tokens, index, "FROM") {
            from = Some(index);
            break;
        }
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let Some(from) = from else {
        return Ok((tokens, Vec::new()));
    };
    let mut end = tokens.len();
    depth = 0;
    for (index, token) in tokens.iter().enumerate().skip(from + 1) {
        if depth == 0
            && [
                "WHERE", "GROUP", "HAVING", "ORDER", "LIMIT", "OFFSET", "FORMAT",
            ]
            .iter()
            .any(|word| keyword_at(&tokens, index, word))
        {
            end = index;
            break;
        }
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let mut virtual_columns = Vec::new();
    let mut index = from + 1;
    depth = 0;
    while index < end {
        if depth == 0 && tokens[index] == Token::LParen {
            if let Some((after, table, relation, virtual_column)) =
                parse_simple_substring_source(&tokens, index)
            {
                let replacement = vec![
                    Token::Word(table),
                    Token::Word("AS".into()),
                    Token::Word(relation),
                ];
                let removed = after - index;
                tokens.splice(index..after, replacement);
                end = end - removed + 3;
                index += 3;
                virtual_columns.push(virtual_column);
                continue;
            }
        }
        match tokens[index] {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
        index += 1;
    }
    Ok((tokens, virtual_columns))
}

fn parse_join_resolved(
    mut tokens: Vec<Token>,
    mut resolve: Option<SchemaResolver<'_>>,
    allow_ordered_comparison_filters: bool,
) -> Result<Option<JoinSelect>, ParseError> {
    // Capture only a top-level HAVING comparison against one scalar SELECT.
    let mut depth = 0usize;
    let mut having = false;
    let mut subquery = None;
    let mut i = 0;
    while i < tokens.len() {
        if depth == 0
            && matches!(&tokens[i], Token::Word(word) if ["ORDER", "LIMIT", "OFFSET", "FORMAT"].iter().any(|clause| word.eq_ignore_ascii_case(clause)))
        {
            having = false;
        }
        if depth == 0
            && matches!(&tokens[i], Token::Word(word) if word.eq_ignore_ascii_case("HAVING"))
        {
            having = true;
        }
        if having
            && depth == 0
            && tokens[i] == Token::LParen
            && i > 0
            && matches!(tokens[i - 1], Token::Operator(_))
            && matches!(tokens.get(i + 1), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT"))
        {
            let start = i + 1;
            let mut end = start;
            let mut nesting = 1usize;
            while end < tokens.len() && nesting > 0 {
                match tokens[end] {
                    Token::LParen => nesting += 1,
                    Token::RParen => nesting -= 1,
                    _ => {}
                }
                end += 1;
            }
            if nesting != 0 {
                return Err(ParseError("HAVING subquery requires ')'".into()));
            }
            let mut inner = tokens[start..end - 1].to_vec();
            let mut level = 0usize;
            let mut from = None;
            let mut alias = false;
            for (index, token) in inner.iter().enumerate() {
                match token {
                    Token::LParen => level += 1,
                    Token::RParen => level = level.saturating_sub(1),
                    Token::Word(word) if level == 0 && word.eq_ignore_ascii_case("FROM") => {
                        from = Some(index);
                        break;
                    }
                    Token::Word(word) if level == 0 && word.eq_ignore_ascii_case("AS") => {
                        alias = true
                    }
                    _ => {}
                }
            }
            let from = from.ok_or_else(|| ParseError("HAVING subquery requires FROM".into()))?;
            if !alias {
                inner.splice(
                    from..from,
                    [
                        Token::Word("AS".into()),
                        Token::Word("__having_scalar".into()),
                    ],
                );
            }
            let rewritten = if let Some(resolver) = resolve.as_mut() {
                let mut proxy = |table: &str| (**resolver)(table);
                implicit::rewrite(inner, Some(&mut proxy))?
            } else {
                implicit::rewrite(inner, None)?
            };
            let inner = parse_join_tokens(rewritten, true)?
                .ok_or_else(|| ParseError("HAVING subquery requires a JOIN".into()))?;
            if (inner.scalar_sum.is_none() && inner.scalar_formula.is_none())
                || inner.grouped_count.is_some()
                || inner.limit.is_some()
                || inner.offset != 0
                || inner.in_subquery.is_some()
            {
                return Err(ParseError(
                    "HAVING subquery requires an ungrouped scalar SUM".into(),
                ));
            }
            subquery = Some(Box::new(inner));
            tokens.splice(i..end, [Token::Number("0".into())]);
            break;
        }
        match tokens[i] {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    let (tokens, virtual_columns) = flatten_simple_substring_sources(tokens)?;
    let rewritten = implicit::rewrite(tokens, resolve)?;
    let mut query =
        parse_join_tokens_internal(rewritten, false, false, allow_ordered_comparison_filters)?;
    if let Some(query) = query.as_mut() {
        query.virtual_columns = virtual_columns;
    }
    if let Some(inner) = subquery {
        let having = query
            .as_mut()
            .and_then(|query| query.grouped_sum_having.as_mut())
            .ok_or_else(|| ParseError("scalar subquery requires grouped SUM HAVING".into()))?;
        having.subquery = Some(inner);
    }
    Ok(query)
}

fn numeric_join_expression(expression: &crate::ScalarExpr) -> bool {
    match expression {
        crate::ScalarExpr::Column(_)
        | crate::ScalarExpr::Integer(_)
        | crate::ScalarExpr::Decimal(_) => true,
        crate::ScalarExpr::TextLength(_)
        | crate::ScalarExpr::TextCompare { .. }
        | crate::ScalarExpr::TextLike { .. }
        | crate::ScalarExpr::IsNull { .. } => true,
        crate::ScalarExpr::TextLiteral(_)
        | crate::ScalarExpr::TextConcat(_)
        | crate::ScalarExpr::TextSubstring { .. } => false,
        crate::ScalarExpr::Round { value, .. } => numeric_join_expression(value),
        crate::ScalarExpr::Case {
            branches,
            else_expr,
        } => {
            branches.iter().all(|(condition, value)| {
                numeric_join_expression(condition) && numeric_join_expression(value)
            }) && else_expr
                .as_ref()
                .is_none_or(|value| numeric_join_expression(value))
        }
        crate::ScalarExpr::Compare(a, _, b)
        | crate::ScalarExpr::And(a, b)
        | crate::ScalarExpr::Or(a, b) => numeric_join_expression(a) && numeric_join_expression(b),
        crate::ScalarExpr::Add(a, b)
        | crate::ScalarExpr::Subtract(a, b)
        | crate::ScalarExpr::Multiply(a, b)
        | crate::ScalarExpr::Divide(a, b)
        | crate::ScalarExpr::Modulo(a, b) => {
            numeric_join_expression(a) && numeric_join_expression(b)
        }
    }
}

fn parse_grouped_sum_projection(parser: &mut Parser) -> Result<(GroupedSum, String), ParseError> {
    if !parser.keyword("SUM") || !parser.take(Token::LParen) {
        return Err(ParseError("JOIN SUM requires '('".into()));
    }
    let start = parser.index;
    let column = projected(parser).ok().filter(|column| {
        column.name != "*" && parser.tokens.get(parser.index) == Some(&Token::RParen)
    });
    let metric = if let Some(column) = column {
        GroupedSum::Column(column)
    } else {
        parser.index = start;
        let expression = crate::expression::parse_expression_tokens(parser)?;
        if !numeric_join_expression(&expression) {
            return Err(ParseError(
                "JOIN SUM requires a numeric arithmetic expression".into(),
            ));
        }
        GroupedSum::Expression(expression)
    };
    if !parser.take(Token::RParen) {
        return Err(ParseError("JOIN grouped SUM requires ')'".into()));
    }
    let name = match &metric {
        GroupedSum::Column(column) => column.display_name(),
        GroupedSum::Expression(expression) => crate::multi_aggregate::render_expr(expression),
    };
    let alias = parser
        .select_output_alias()?
        .unwrap_or_else(|| format!("sum({name})"));
    Ok((metric, alias))
}

fn parse_grouped_window_average(
    parser: &mut Parser,
) -> Result<Option<GroupedWindowAverage>, ParseError> {
    if !matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("AVG"))
        || parser.tokens.get(parser.index + 1) != Some(&Token::LParen)
        || !matches!(parser.tokens.get(parser.index + 2), Some(Token::Word(name)) if name.eq_ignore_ascii_case("SUM"))
    {
        return Ok(None);
    }
    parser.keyword("AVG");
    if !parser.take(Token::LParen) || !parser.keyword("SUM") || !parser.take(Token::LParen) {
        return Err(ParseError(
            "grouped window AVG requires AVG(SUM(column))".into(),
        ));
    }
    let value = projected(parser)?;
    if value.name == "*" || !parser.take(Token::RParen) || !parser.take(Token::RParen) {
        return Err(ParseError(
            "grouped window AVG requires one SUM column".into(),
        ));
    }
    if !parser.keyword("OVER")
        || !parser.take(Token::LParen)
        || !parser.keyword("PARTITION")
        || !parser.keyword("BY")
    {
        return Err(ParseError(
            "grouped window AVG requires PARTITION BY".into(),
        ));
    }
    let mut partition = vec![projected(parser)?];
    while parser.take(Token::Comma) {
        if partition.len() == 8 {
            return Err(ParseError(
                "grouped window AVG supports at most 8 partition keys".into(),
            ));
        }
        partition.push(projected(parser)?);
    }
    if partition.iter().any(|key| key.name == "*") || !parser.take(Token::RParen) {
        return Err(ParseError(
            "grouped window AVG requires partition columns".into(),
        ));
    }
    let alias = parser
        .select_output_alias()?
        .unwrap_or_else(|| format!("avg(sum({}))", value.display_name()));
    Ok(Some(GroupedWindowAverage {
        value,
        partition,
        alias,
    }))
}

fn projected_or_cast(parser: &mut Parser) -> Result<JoinColumn, ParseError> {
    if !parser.keyword("CAST") {
        return projected(parser);
    }
    if !parser.take(Token::LParen) {
        return Err(ParseError("CAST requires '('".into()));
    }
    let column = projected(parser)?;
    if column.name == "*" || !parser.keyword("AS") || parser.index >= parser.tokens.len() {
        return Err(ParseError("CAST requires a column and type".into()));
    }
    let mut nested = 0usize;
    loop {
        match parser.tokens.get(parser.index) {
            Some(Token::LParen) => nested += 1,
            Some(Token::RParen) if nested == 0 => {
                parser.index += 1;
                break;
            }
            Some(Token::RParen) => nested -= 1,
            Some(_) => {}
            None => return Err(ParseError("CAST requires closing ')'".into())),
        }
        parser.index += 1;
    }
    Ok(column)
}

fn parse_join_formula(
    parser: &mut Parser,
) -> Result<Option<(JoinAggregateFormula, String)>, ParseError> {
    let start = parser.index;
    let mut end = start;
    let mut depth = 0usize;
    let mut has_operator = false;
    let mut has_sum = false;
    let mut has_comma = false;
    while let Some(token) = parser.tokens.get(end) {
        match token {
            Token::Word(word)
                if depth == 0
                    && (word.eq_ignore_ascii_case("AS") || word.eq_ignore_ascii_case("FROM")) =>
            {
                break;
            }
            Token::Word(word)
                if word.eq_ignore_ascii_case("SUM")
                    && parser.tokens.get(end + 1) == Some(&Token::LParen) =>
            {
                has_sum = true
            }
            Token::Comma if depth == 0 => has_comma = true,
            Token::Plus | Token::Minus | Token::Star | Token::Slash | Token::Percent
                if depth == 0 =>
            {
                has_operator = true
            }
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
        end += 1;
    }
    if !has_sum || !has_operator || has_comma {
        return Ok(None);
    }
    let mut rewritten = Vec::new();
    let mut sums = Vec::new();
    let mut index = start;
    while index < end {
        if matches!(&parser.tokens[index], Token::Word(word) if word.eq_ignore_ascii_case("SUM"))
            && parser.tokens.get(index + 1) == Some(&Token::LParen)
        {
            if sums.len() >= 8 {
                return Err(ParseError(
                    "JOIN aggregate formula supports at most 8 SUMs".into(),
                ));
            }
            let begin = index + 2;
            let mut cursor = begin;
            let mut nesting = 1usize;
            while cursor < end && nesting > 0 {
                match &parser.tokens[cursor] {
                    Token::LParen => nesting += 1,
                    Token::RParen => nesting -= 1,
                    _ => {}
                }
                cursor += 1;
            }
            if nesting != 0 {
                return Err(ParseError("JOIN SUM requires ')'".into()));
            }
            let source = &parser.tokens[begin..cursor - 1];
            let mut encoded = Vec::new();
            let mut position = 0;
            while position < source.len() {
                let identifier = |token: &Token| match token {
                    Token::Word(value) | Token::QuotedWord(value) => Some(value.clone()),
                    _ => None,
                };
                if let (Some(left), Some(Token::Dot), Some(right)) = (
                    source.get(position).and_then(identifier),
                    source.get(position + 1),
                    source.get(position + 2).and_then(identifier),
                ) {
                    encoded.push(Token::Word(format!("{left}\0{right}")));
                    position += 3;
                } else {
                    encoded.push(source[position].clone());
                    position += 1;
                }
            }
            let mut inner = Parser {
                tokens: encoded,
                index: 0,
            };
            let expression = crate::expression::parse_expression_tokens(&mut inner)?;
            if inner.index != inner.tokens.len() || !numeric_join_expression(&expression) {
                return Err(ParseError("invalid JOIN SUM expression".into()));
            }
            rewritten.push(Token::Word(format!("__join_sum_{}", sums.len())));
            sums.push(expression);
            index = cursor;
        } else {
            if matches!(&parser.tokens[index], Token::Word(word) if word.starts_with("__join_sum_"))
            {
                return Err(ParseError(
                    "JOIN aggregate formula uses a reserved name".into(),
                ));
            }
            rewritten.push(parser.tokens[index].clone());
            index += 1;
        }
    }
    let mut formula_parser = Parser {
        tokens: rewritten,
        index: 0,
    };
    let expression = crate::expression::parse_expression_tokens(&mut formula_parser)?;
    fn arithmetic(expression: &crate::ScalarExpr) -> bool {
        match expression {
            crate::ScalarExpr::Column(_)
            | crate::ScalarExpr::Integer(_)
            | crate::ScalarExpr::Decimal(_) => true,
            crate::ScalarExpr::Add(a, b)
            | crate::ScalarExpr::Subtract(a, b)
            | crate::ScalarExpr::Multiply(a, b)
            | crate::ScalarExpr::Divide(a, b)
            | crate::ScalarExpr::Modulo(a, b) => arithmetic(a) && arithmetic(b),
            _ => false,
        }
    }
    if !arithmetic(&expression) {
        return Err(ParseError(
            "JOIN aggregate formula requires arithmetic".into(),
        ));
    }
    let mut columns = Vec::new();
    expression.referenced_columns(&mut columns);
    if formula_parser.index != formula_parser.tokens.len()
        || columns
            .iter()
            .any(|name| !(0..sums.len()).any(|index| name == &format!("__join_sum_{index}")))
    {
        return Err(ParseError(
            "JOIN aggregate formula can only use SUM results and numeric constants".into(),
        ));
    }
    parser.index = end;
    if !parser.keyword("AS") {
        return Err(ParseError(
            "JOIN aggregate formula requires an alias".into(),
        ));
    }
    let alias = parser.bounded_word()?;
    Ok(Some((JoinAggregateFormula { sums, expression }, alias)))
}

/// Parse SUM(value) * multiplier / SUM(SUM(value)) OVER (PARTITION BY key).
/// The execution path accumulates the partition total while it groups the
/// joined rows, so this form stays bounded by the existing grouped-row cap.
fn parse_grouped_window_ratio(
    parser: &mut Parser,
    grouped_sum: Option<&GroupedSum>,
) -> Result<Option<GroupedWindowRatio>, ParseError> {
    let mut candidate = Parser {
        tokens: parser.tokens.clone(),
        index: parser.index,
    };
    if !candidate.keyword("SUM") || !candidate.take(Token::LParen) {
        return Ok(None);
    }
    let Ok(numerator) = projected(&mut candidate) else {
        return Ok(None);
    };
    if numerator.name == "*" || !candidate.take(Token::RParen) || !candidate.take(Token::Star) {
        return Ok(None);
    }
    let Some(Token::Number(multiplier)) = candidate.tokens.get(candidate.index) else {
        return Ok(None);
    };
    let multiplier = multiplier.clone();
    candidate.index += 1;
    if !candidate.take(Token::Slash)
        || !candidate.keyword("SUM")
        || !candidate.take(Token::LParen)
        || !candidate.keyword("SUM")
        || !candidate.take(Token::LParen)
    {
        return Ok(None);
    }
    let Ok(denominator) = projected(&mut candidate) else {
        return Ok(None);
    };
    if denominator != numerator
        || !candidate.take(Token::RParen)
        || !candidate.take(Token::RParen)
        || !candidate.keyword("OVER")
        || !candidate.take(Token::LParen)
        || !candidate.keyword("PARTITION")
        || !candidate.keyword("BY")
    {
        return Ok(None);
    }
    let partition = projected(&mut candidate)?;
    if partition.name == "*" || !candidate.take(Token::RParen) {
        return Ok(None);
    }
    let alias = candidate
        .select_output_alias()?
        .ok_or_else(|| ParseError("grouped window ratio requires an output alias".into()))?;
    if !matches!(candidate.tokens.get(candidate.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("FROM"))
    {
        return Err(ParseError(
            "grouped window ratio must be the final SELECT expression".into(),
        ));
    }
    if grouped_sum != Some(&GroupedSum::Column(numerator)) {
        return Err(ParseError(
            "grouped window ratio must use the selected grouped SUM".into(),
        ));
    }
    parser.index = candidate.index;
    Ok(Some(GroupedWindowRatio {
        partition,
        multiplier,
        alias,
    }))
}

type JoinedSubqueryFilter = (JoinInSubquery, Vec<Vec<crate::Predicate>>);

fn parse_distinct_not_in(parser: &mut Parser) -> Result<Option<JoinedSubqueryFilter>, ParseError> {
    let mut end = parser.index;
    let mut depth = 0usize;
    let mut between = false;
    let mut has_top_or = false;
    let mut pieces = Vec::new();
    let mut piece_start = end;
    while let Some(token) = parser.tokens.get(end) {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(word)
                if depth == 0
                    && ["GROUP", "ORDER", "LIMIT", "OFFSET", "HAVING"]
                        .iter()
                        .any(|name| word.eq_ignore_ascii_case(name)) =>
            {
                break;
            }
            Token::Semicolon if depth == 0 => break,
            Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("OR") => has_top_or = true,
            Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("BETWEEN") => {
                between = true
            }
            Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("AND") => {
                if between {
                    between = false;
                } else {
                    pieces.push(&parser.tokens[piece_start..end]);
                    piece_start = end + 1;
                }
            }
            _ => {}
        }
        end += 1;
    }
    pieces.push(&parser.tokens[piece_start..end]);
    if pieces.iter().any(|piece| piece.is_empty()) {
        return Err(ParseError("JOIN WHERE requires complete predicates".into()));
    }
    let mut found = None;
    let mut remainder = Vec::new();
    for piece in pieces {
        let candidate = piece.len() >= 6
            && matches!(&piece[0], Token::Word(_) | Token::QuotedWord(_))
            && matches!(&piece[1], Token::Word(word) if word.eq_ignore_ascii_case("NOT"))
            && matches!(&piece[2], Token::Word(word) if word.eq_ignore_ascii_case("IN"))
            && piece[3] == Token::LParen
            && matches!(&piece[4], Token::Word(word) if word.eq_ignore_ascii_case("SELECT"))
            && piece.last() == Some(&Token::RParen);
        if candidate {
            if found.is_some() {
                return Err(ParseError("JOIN supports one NOT IN subquery".into()));
            }
            let name = match &piece[0] {
                Token::Word(name) | Token::QuotedWord(name) => name.clone(),
                _ => unreachable!(),
            };
            let inner = crate::parse_select_tokens(piece[4..piece.len() - 1].to_vec())?;
            if !matches!(&inner.source, Some(crate::Source::Table(_)))
                || !matches!(&inner.projection, crate::Projection::Columns(columns) if columns.len() == 1)
                || inner
                    .predicates
                    .iter()
                    .chain(inner.or_predicates.iter().flatten())
                    .any(|predicate| matches!(predicate, crate::Predicate::ColumnComparison { .. }))
            {
                return Err(ParseError(
                    "JOIN NOT IN requires an uncorrelated single-column subquery".into(),
                ));
            }
            found = Some(JoinInSubquery {
                column: JoinColumn {
                    relation: String::new(),
                    name,
                },
                inner: Box::new(inner),
                negated: true,
            });
        } else {
            if !remainder.is_empty() {
                remainder.push(Token::Word("AND".into()));
            }
            remainder.extend_from_slice(piece);
        }
    }
    let Some(subquery) = found else {
        return Ok(None);
    };
    if has_top_or {
        return Err(ParseError(
            "JOIN NOT IN subquery requires a conjunctive WHERE".into(),
        ));
    }
    let branches = if remainder.is_empty() {
        vec![vec![]]
    } else {
        let mut rest = Parser {
            tokens: remainder,
            index: 0,
        };
        let branches = crate::where_parser::parse_where(&mut rest)?;
        if rest.index != rest.tokens.len() {
            return Err(ParseError("invalid JOIN WHERE after NOT IN".into()));
        }
        branches
    };
    if branches.len() > 1 {
        return Err(ParseError(
            "JOIN NOT IN subquery requires a conjunctive WHERE".into(),
        ));
    }
    parser.index = end;
    Ok(Some((subquery, branches)))
}

/// Parse a bounded ZIP-like prefix expression in a JOIN filter.
fn parse_join_substring(parser: &mut Parser) -> Result<(JoinColumn, usize, usize), ParseError> {
    if !(parser.keyword("SUBSTR") || parser.keyword("SUBSTRING")) || !parser.take(Token::LParen) {
        return Err(ParseError("expected SUBSTR or SUBSTRING(".into()));
    }
    let column = projected(parser)?;
    if column.name == "*" {
        return Err(ParseError("SUBSTRING requires a source column".into()));
    }
    let comma = parser.take(Token::Comma);
    if !comma && !parser.keyword("FROM") {
        return Err(ParseError("SUBSTRING requires FROM or comma".into()));
    }
    let start = bound(parser, "SUBSTRING start")?;
    if !(1..=64).contains(&start) {
        return Err(ParseError("SUBSTRING start must be in 1..64".into()));
    }
    if comma {
        if !parser.take(Token::Comma) {
            return Err(ParseError("SUBSTR requires a length".into()));
        }
    } else if !parser.keyword("FOR") {
        return Err(ParseError("SUBSTRING requires FOR".into()));
    }
    let length = bound(parser, "SUBSTRING length")?;
    if length == 0 || length > 64 || !parser.take(Token::RParen) {
        return Err(ParseError(
            "SUBSTRING length must be in 1..64 with closing ')'".into(),
        ));
    }
    Ok((column, start, length))
}

fn grouped_case_value(metric: crate::GroupedMetricValue) -> GroupedCaseValue {
    match metric {
        crate::GroupedMetricValue::Count => GroupedCaseValue::Count,
        crate::GroupedMetricValue::Column(column) => {
            let (relation, name) = column.split_once('\0').unwrap_or(("", column.as_str()));
            GroupedCaseValue::Column(JoinColumn {
                relation: relation.to_owned(),
                name: name.to_owned(),
            })
        }
        crate::GroupedMetricValue::Expression(expression) => {
            GroupedCaseValue::Expression(expression)
        }
    }
}

fn parse_global_case_metrics(
    parser: &mut Parser,
    metrics: &mut Vec<(crate::GroupedFilter, GroupedCaseValue, String)>,
) -> Result<(), ParseError> {
    loop {
        if metrics.len() >= MAX_JOIN_GROUPED_CASE_METRICS {
            return Err(ParseError(format!(
                "JOIN supports at most {MAX_JOIN_GROUPED_CASE_METRICS} grouped CASE metrics"
            )));
        }
        if !parser.keyword("SUM") || !parser.take(Token::LParen) {
            return Err(ParseError("global JOIN CASE expects SUM(CASE ...)".into()));
        }
        let (metric, filter) = super::grouped_metric_value(parser, crate::AggregateFunction::Sum)?;
        if !matches!(
            filter,
            Some(
                crate::GroupedFilter::CaseElseZero { .. }
                    | crate::GroupedFilter::CaseElseZeroCombined { .. }
                    | crate::GroupedFilter::CaseExpression { .. }
                    | crate::GroupedFilter::CaseNull { .. }
                    | crate::GroupedFilter::Comparison { .. }
            )
        ) {
            return Err(ParseError(
                "JOIN grouped SUM requires a supported CASE expression".into(),
            ));
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError(
                "JOIN grouped CASE requires a closing ')'".into(),
            ));
        }
        parser.keyword("AS");
        metrics.push((
            filter.expect("CASE filter checked"),
            grouped_case_value(metric),
            parser.output_alias_word()?,
        ));
        if !parser.take(Token::Comma) {
            return Ok(());
        }
    }
}

fn parse_join_tokens(
    tokens: Vec<Token>,
    scalar_subquery: bool,
) -> Result<Option<JoinSelect>, ParseError> {
    parse_join_tokens_internal(tokens, scalar_subquery, false, false)
}

pub(super) fn parse_correlated_avg_inner_tokens(
    tokens: Vec<Token>,
) -> Result<Option<JoinSelect>, ParseError> {
    parse_join_tokens_internal(tokens, false, true, false)
}


pub fn explain_join(query: &JoinSelect) -> String {
    let conditions = query
        .keys
        .iter()
        .map(|(left, right)| {
            format!(
                "{}.{} = {}.{}",
                left.relation, left.name, right.relation, right.name
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    let predicate = if query.keys.is_empty() && query.using_keys.is_empty() {
        "cartesian product".to_owned()
    } else if query.using_keys.is_empty() {
        format!("ON {conditions}")
    } else {
        format!("USING ({})", query.using_keys.join(", "))
    };
    let mut plan = format!(
        "Source: table {} AS {}\nSource: table {} AS {}\nJoin: {} {}",
        query.left,
        query.left_alias,
        query.right,
        query.right_alias,
        if query.keys.is_empty() && query.using_keys.is_empty() {
            "Cross".to_owned()
        } else {
            format!("{}{:?}", if query.any { "Any " } else { "" }, query.kind)
        },
        predicate,
    );
    for step in &query.steps {
        let conditions = step
            .keys
            .iter()
            .map(|(left, right)| {
                format!(
                    "{}.{} = {}.{}",
                    left.relation, left.name, right.relation, right.name
                )
            })
            .collect::<Vec<_>>()
            .join(" AND ");
        let predicate = if step.keys.is_empty() && step.using_keys.is_empty() {
            "cartesian product".to_owned()
        } else if step.using_keys.is_empty() {
            format!("ON {conditions}")
        } else {
            format!("USING ({})", step.using_keys.join(", "))
        };
        plan.push_str(&format!(
            "\nSource: table {} AS {}\nJoin: {} {}",
            step.table,
            step.alias,
            if step.keys.is_empty() && step.using_keys.is_empty() {
                "Cross".to_owned()
            } else {
                format!("{}{:?}", if step.any { "Any " } else { "" }, step.kind)
            },
            predicate
        ));
    }
    plan.push_str(&format!(
        "\nProject: {}\nLimit: {}",
        if !query.scalar_aggregates.is_empty() {
            query
                .scalar_aggregates
                .iter()
                .map(|item| {
                    format!(
                        "{} AS {}",
                        join_scalar_aggregate_header(&item.aggregate, item.distinct),
                        item.alias
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        } else if let Some(aggregate) = &query.aggregate {
            match aggregate {
                JoinAggregate::CountStar => "count(*)".to_owned(),
                JoinAggregate::CountColumn(column) => {
                    format!("count({})", column.display_name())
                }
                JoinAggregate::Numeric { function, column } => {
                    format!("{}({})", function.name(), column.display_name())
                }
            }
        } else {
            query.columns.as_ref().map_or("*".to_owned(), |columns| {
                columns
                    .iter()
                    .map(|col| {
                        if col.relation.is_empty() {
                            col.name.clone()
                        } else {
                            format!("{}.{}", col.relation, col.name)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
        },
        query
            .limit
            .map_or("unbounded".to_owned(), |n| n.to_string())
    ));
    if !query.using_keys.is_empty() {
        plan.push_str(&format!("\nUsing: {}", query.using_keys.join(", ")));
    }
    if let Some((column, not_null)) = &query.where_null {
        plan.push_str(&format!(
            "\nFilter: {} IS {}NULL",
            if column.relation.is_empty() {
                column.name.clone()
            } else {
                format!("{}.{}", column.relation, column.name)
            },
            if *not_null { "NOT " } else { "" }
        ));
    }
    if let Some((column, operator, value)) = &query.where_comparison {
        plan.push_str(&format!(
            "\nFilter: {} {:?} literal({})",
            if column.relation.is_empty() {
                column.name.clone()
            } else {
                format!("{}.{}", column.relation, column.name)
            },
            operator,
            value
        ));
    }
    for (column, operator, value) in &query.where_comparisons_extra {
        plan.push_str(&format!(
            "\nFilter: {} {:?} literal({})",
            column.display_name(),
            operator,
            value
        ));
    }
    for (left, operator, right) in &query.where_column_comparisons {
        plan.push_str(&format!(
            "\nFilter: {} {:?} {}",
            left.display_name(),
            operator,
            right.display_name()
        ));
    }
    for (column, values) in &query.where_in_filters {
        plan.push_str(&format!(
            "\nFilter: {} IN ({} values)",
            column.display_name(),
            values.len()
        ));
    }
    for (column, _, case_insensitive, negated) in &query.where_like_filters {
        plan.push_str(&format!(
            "\nFilter: {} {}{} pattern",
            column.display_name(),
            if *negated { "NOT " } else { "" },
            if *case_insensitive { "ILIKE" } else { "LIKE" }
        ));
    }
    if !query.order_by.is_empty() {
        let terms = query
            .order_by
            .iter()
            .map(|(column, descending, nulls_first)| {
                let placement = match nulls_first {
                    Some(true) => " NULLS FIRST",
                    Some(false) => " NULLS LAST",
                    None => "",
                };
                let name = if column.is_count_order() {
                    query
                        .aggregate_alias
                        .as_deref()
                        .unwrap_or("count")
                        .to_owned()
                } else {
                    if column.relation.is_empty() {
                        column.name.clone()
                    } else {
                        format!("{}.{}", column.relation, column.name)
                    }
                };
                format!(
                    "{name} {}{}",
                    if *descending { "DESC" } else { "ASC" },
                    placement
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        plan.push_str(&format!("\nOrder: {terms}"));
    }
    if let Some(subquery) = &query.in_subquery {
        plan.push_str(&format!(
            "\nFilter: {} {}IN subquery",
            subquery.column.display_name(),
            if subquery.negated { "NOT " } else { "" }
        ));
    }
    if let Some(branches) = &query.scalar_where {
        plan.push_str(&format!("\nFilter: {} boolean branches", branches.len()));
    }
    if let Some((column, pattern, negated)) = &query.on_right_like {
        plan.push_str(&format!(
            "\nJoin filter: {} {}LIKE '{}'",
            column.display_name(),
            if *negated { "NOT " } else { "" },
            pattern.replace('\'', "''")
        ));
    }
    if let Some(having) = &query.grouped_sum_having {
        let value = if having.subquery.is_some() {
            "scalar subquery"
        } else {
            &having.value
        };
        plan.push_str(&format!("\nHaving: SUM {:?} {value}", having.operator));
    }
    if let Some(expression) = &query.scalar_sum {
        plan.push_str(&format!(
            "\nAggregate: SUM({})",
            crate::multi_aggregate::render_expr(expression)
        ));
    }
    if let Some((position, alias)) = &query.grouped_year {
        if let Some(column) = query
            .columns
            .as_ref()
            .and_then(|columns| columns.get(*position))
        {
            plan.push_str(&format!(
                "\nGroup key: EXTRACT(YEAR FROM {}) AS {alias}",
                column.display_name()
            ));
        }
    }
    if let Some(columns) = &query.grouped_count {
        let keys = columns
            .iter()
            .map(JoinColumn::display_name)
            .collect::<Vec<_>>()
            .join(", ");
        if let Some(metric) = &query.grouped_sum {
            let expression = match metric {
                GroupedSum::Column(column) => column.display_name(),
                GroupedSum::Expression(expression) => {
                    crate::multi_aggregate::render_expr(expression)
                }
            };
            plan.push_str(&format!(
                "\nAggregate: GROUP BY {keys} with SUM({expression})"
            ));
        } else if let Some(formula) = &query.grouped_formula {
            plan.push_str(&format!(
                "\nAggregate: GROUP BY {keys} with {} SUM expressions",
                formula.sums.len()
            ));
        } else if let Some(column) = &query.grouped_min {
            plan.push_str(&format!(
                "\nAggregate: GROUP BY {keys} with MIN({})",
                column.display_name()
            ));
        } else if let Some(column) = &query.grouped_distinct {
            plan.push_str(&format!(
                "\nAggregate: GROUP BY {keys} with COUNT(DISTINCT {})",
                column.display_name()
            ));
        } else if let Some(column) = &query.grouped_count_column {
            plan.push_str(&format!(
                "\nAggregate: GROUP BY {keys} with COUNT({})",
                column.display_name()
            ));
        } else if query.grouped_case_counts.is_empty() {
            plan.push_str(&format!("\nAggregate: GROUP BY {keys} with COUNT(*)"));
        } else {
            let aliases = query
                .grouped_case_counts
                .iter()
                .map(|(_, _, alias)| alias.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            plan.push_str(&format!(
                "\nAggregate: GROUP BY {keys} with conditional sums ({aliases})"
            ));
        }
    }
    if query.offset > 0 {
        plan.push_str(&format!("\nOffset: {}", query.offset));
    }
    if query.steps.is_empty() {
        plan.push_str(&format!(
            "\nExecution bounds: buffered input <= {} rows/side; streaming INNER build <= {} rows; result <= {} rows; build budget {} MiB",
            MAX_BUFFERED_JOIN_ROWS,
            MAX_STREAM_JOIN_BUILD_ROWS,
            MAX_JOIN_RESULT_ROWS,
            MAX_JOIN_BUILD_BYTES / (1024 * 1024)
        ));
    } else {
        plan.push_str(&format!(
            "\nExecution bounds: chained intermediate <= {} rows and {} MiB; individual joins retain input limits",
            MAX_JOIN_RESULT_ROWS,
            MAX_JOIN_CHAIN_BYTES / (1024 * 1024)
        ));
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explain_uses_the_same_resource_limits_as_join_execution() {
        let query = parse_join("SELECT * FROM a x JOIN b y ON x.id = y.id")
            .unwrap()
            .unwrap();
        let plan = explain_join(&query);
        assert!(plan.contains("buffered input <= 32768 rows/side"), "{plan}");
        assert!(
            plan.contains("streaming INNER build <= 262144 rows"),
            "{plan}"
        );
        assert!(plan.contains("build budget 64 MiB"), "{plan}");

        let chained =
            parse_join("SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON y.id = z.id")
                .unwrap()
                .unwrap();
        let chain_plan = explain_join(&chained);
        assert!(
            chain_plan.contains("chained intermediate <= 131072 rows and 128 MiB"),
            "{chain_plan}"
        );
    }

    #[test]
    fn chained_join_accepts_nested_on_equalities() {
        let query = parse_join(
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON (y.id = z.id AND (x.k = z.k))",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.steps[0].keys.len(), 2);
        for statement in [
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON (y.id = z.id",
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON (y.id = z.id OR x.k = z.k)",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
    }

    #[test]
    fn chained_join_rejects_equalities_on_the_same_side() {
        for statement in [
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON x.id = y.id",
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON z.id = z.other",
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON z.id = x.id AND x.k = y.k",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
        assert!(
            parse_join("SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON y.id = z.id",)
                .is_ok()
        );
    }

    #[test]
    fn explain_shows_the_original_using_predicate() {
        let query = parse_join("SELECT * FROM a x FULL JOIN b y USING (id, tenant)")
            .unwrap()
            .unwrap();
        let plan = explain_join(&query);
        assert!(plan.contains("Join: Full USING (id, tenant)"), "{plan}");
        assert!(!plan.contains("Join: Full ON"), "{plan}");
    }

    #[test]
    fn chained_join_rejects_unknown_on_qualifiers_at_parse_time() {
        for statement in [
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON missing.id = z.id",
            "SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON z.id = missing.id",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
        assert!(
            parse_join("SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c z ON x.id = z.id",)
                .is_ok()
        );
    }

    #[test]
    fn explain_lists_chained_sources_before_project_and_limit() {
        let query = parse_join(
            "SELECT x.id FROM a x JOIN b y ON x.id = y.id JOIN c z ON y.id = z.id LIMIT 3",
        )
        .unwrap()
        .unwrap();
        let plan = explain_join(&query);
        let step = plan.find("Source: table c AS z").unwrap();
        let project = plan.find("Project: x.id").unwrap();
        let limit = plan.find("Limit: 3").unwrap();
        assert!(step < project && project < limit, "{plan}");
    }

    #[test]
    fn chained_joins_accept_explicit_all_strictness() {
        for statement in [
            "SELECT * FROM a x JOIN b y ON x.id = y.id ALL JOIN c z ON y.id = z.id",
            "SELECT * FROM a x JOIN b y ON x.id = y.id INNER ALL JOIN c z ON y.id = z.id",
            "SELECT * FROM a x JOIN b y ON x.id = y.id LEFT ALL JOIN c z USING (id)",
        ] {
            let query = parse_join(statement).unwrap().unwrap();
            assert_eq!(query.steps.len(), 1);
            assert!(!query.steps[0].any, "{statement}");
        }
        for statement in [
            "SELECT * FROM a x JOIN b y ON x.id = y.id ALL ALL JOIN c z USING (id)",
            "SELECT * FROM a x JOIN b y ON x.id = y.id ANY INNER ANY JOIN c z USING (id)",
            "SELECT * FROM a x JOIN b y ON x.id = y.id ALL INNER ANY JOIN c z USING (id)",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
    }

    #[test]
    fn explain_includes_using_filter_sort_and_offset() {
        let query = parse_join(
            "SELECT a.id FROM a FULL JOIN b USING (id) WHERE b.id IS NULL ORDER BY a.id DESC NULLS LAST LIMIT 5 OFFSET 2",
        )
        .unwrap()
        .unwrap();
        let plan = explain_join(&query);
        for expected in [
            "Using: id",
            "Filter: b.id IS NULL",
            "Order: a.id DESC NULLS LAST",
            "Offset: 2",
        ] {
            assert!(plan.contains(expected), "{plan}");
        }
    }

    #[test]
    fn explain_lists_multiple_join_sort_columns() {
        let query = parse_join(
            "SELECT a.id, b.name FROM a JOIN b ON a.id = b.id ORDER BY a.id ASC, b.name DESC NULLS LAST LIMIT 3",
        )
        .unwrap()
        .unwrap();
        let plan = explain_join(&query);
        assert!(
            plan.contains("Order: a.id ASC, b.name DESC NULLS LAST"),
            "{plan}"
        );
    }

    #[test]
    fn parses_chained_using_keys_and_rejects_duplicates() {
        let query = parse_join(
            "SELECT * FROM facts f JOIN customers c ON f.customer_id = c.id JOIN regions r USING (region_id)",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.steps[0].using_keys, ["region_id"]);
        assert!(query.steps[0].keys.is_empty());
        let query = parse_join(
            "SELECT * FROM facts f JOIN customers c USING (customer_id) JOIN regions r ON c.customer_id = r.customer_id",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.using_keys, ["customer_id"]);
        assert!(parse_join(
            "SELECT * FROM facts f JOIN customers c ON f.id = c.id JOIN regions r USING (id, id)"
        )
        .is_err());
    }

    #[test]
    fn parses_chained_any_join_strictness() {
        let query = parse_join(
            "SELECT * FROM a x LEFT ANY JOIN b y ON x.id = y.id ANY LEFT JOIN c z ON y.id = z.id",
        )
        .unwrap()
        .unwrap();
        assert!(query.any);
        assert!(query.steps[0].any);
        assert_eq!(query.steps[0].kind, JoinType::Left);
        assert!(
            parse_join(
                "SELECT * FROM a x JOIN b y ON x.id = y.id FULL ANY JOIN c z ON y.id = z.id"
            )
            .is_err()
        );
        let right_any = parse_join(
            "SELECT * FROM a x RIGHT ANY JOIN b y ON x.id = y.id LEFT JOIN c z ON y.id = z.id",
        )
        .unwrap()
        .unwrap();
        assert_eq!(right_any.kind, JoinType::Right);
        assert!(right_any.any);
        assert_eq!(right_any.steps.len(), 1);
        assert!(!right_any.steps[0].any);
    }

    #[test]
    fn parses_chained_right_and_full_joins() {
        let query = parse_join(
            "SELECT o.id, r.id FROM orders o JOIN customers c ON o.id = c.id RIGHT JOIN regions r ON c.id = r.id",
        )
        .unwrap().unwrap();
        assert_eq!(query.steps[0].kind, JoinType::Right);
        let query = parse_join(
            "SELECT * FROM orders o FULL JOIN customers c ON o.id = c.id FULL OUTER JOIN regions r ON c.id = r.id",
        )
        .unwrap().unwrap();
        assert_eq!(query.kind, JoinType::Full);
        assert_eq!(query.steps[0].kind, JoinType::Full);
    }

    #[test]
    fn parses_chained_left_joins() {
        let query = parse_join(
            "SELECT o.id, r.label FROM orders o LEFT JOIN customers c ON o.customer_id = c.id LEFT OUTER JOIN regions r ON c.region_id = r.id",
        )
        .unwrap().unwrap();
        assert_eq!(query.kind, JoinType::Left);
        assert_eq!(query.steps[0].kind, JoinType::Left);
        assert_eq!(query.steps[0].keys[0].0.relation, "c");
    }

    #[test]
    fn validates_projected_aliases_after_parsing_join_chain() {
        let valid = "SELECT r.label FROM orders o JOIN customers c ON o.customer_id = c.id JOIN regions r ON c.region_id = r.id";
        assert_eq!(
            parse_join(valid).unwrap().unwrap().columns.unwrap()[0].relation,
            "r"
        );
        let invalid = "SELECT z.label FROM orders o JOIN customers c ON o.customer_id = c.id JOIN regions r ON c.region_id = r.id";
        assert!(parse_join(invalid).is_err());
    }

    #[test]
    fn parses_three_table_inner_joins() {
        let query = parse_join("SELECT o.id, c.name, r.label FROM orders o JOIN customers c ON o.customer_id = c.id JOIN regions r ON c.region_id = r.id ORDER BY o.id LIMIT 5").unwrap().unwrap();
        assert_eq!(query.steps.len(), 1);
        assert_eq!(query.steps[0].table, "regions");
        assert_eq!(query.steps[0].keys[0].0.relation, "c");
        assert_eq!(query.order_by[0].0.relation, "o");
        assert!(
            parse_join("SELECT * FROM a x JOIN b y ON x.id = y.id JOIN c x ON y.id = x.id")
                .is_err()
        );
        assert!(
            parse_join(
                "SELECT * FROM a x JOIN b y ON x.id = y.id RIGHT ANY JOIN c z ON y.id = z.id"
            )
            .is_ok()
        );
    }

    #[test]
    fn repeated_on_predicates_do_not_exhaust_unique_key_limit() {
        let repeated = (0..12)
            .map(|i| {
                if i % 2 == 0 {
                    "a.id = b.id"
                } else {
                    "b.id = a.id"
                }
            })
            .collect::<Vec<_>>()
            .join(" AND ");
        let query = parse_join(&format!("SELECT * FROM l a JOIN r b ON {repeated}"))
            .unwrap()
            .unwrap();
        assert_eq!(query.keys.len(), 1);
        let distinct = (0..9)
            .map(|i| format!("a.k{i} = b.k{i}"))
            .collect::<Vec<_>>()
            .join(" AND ");
        assert!(parse_join(&format!("SELECT * FROM l a JOIN r b ON {distinct}")).is_err());
    }

    #[test]
    fn rejects_unknown_filter_and_sort_qualifiers_during_parse() {
        for statement in [
            "SELECT * FROM l a JOIN r b ON a.id = b.id WHERE c.id IS NULL",
            "SELECT * FROM l a JOIN r b ON a.id = b.id ORDER BY c.id",
            "SELECT * FROM l a JOIN r b ON a.id = b.id ORDER BY a.id, c.id DESC",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
        assert!(
            parse_join(
                "SELECT * FROM l a JOIN r b ON a.id = b.id WHERE b.id IS NOT NULL ORDER BY a.id"
            )
            .is_ok()
        );
        assert!(
            parse_join(
                "SELECT * FROM l a JOIN r b ON a.id = b.id JOIN x c ON b.id = c.id ORDER BY c.id"
            )
            .is_ok()
        );
    }

    #[test]
    fn parses_inner_joins_with_aliases_and_bounds() {
        let query = parse_join(
            "SELECT u.id, p.label FROM users AS u INNER JOIN plans p ON u.plan_id = p.id ORDER BY u.id DESC LIMIT 5 OFFSET 2;",
        )
        .unwrap().unwrap();
        assert_eq!(query.left, "users");
        assert_eq!(query.right_alias, "p");
        assert_eq!(query.keys[0].0.name, "plan_id");
        assert_eq!(query.limit, Some(5));
        assert_eq!(query.offset, 2);
        assert!(parse_join("SELECT id FROM users").unwrap().is_none());
    }

    #[test]
    fn parses_composite_join_predicates() {
        let query = parse_join(
            "SELECT a.id, b.id FROM first a LEFT JOIN second b ON a.tenant = b.tenant AND b.user_id = a.user_id ORDER BY a.id LIMIT 2",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.keys.len(), 2);
        assert_eq!(query.keys[1].0.relation, "b");
        let excess = format!(
            "SELECT * FROM a JOIN b ON {}",
            (0..9)
                .map(|index| format!("a.k{index} = b.k{index}"))
                .collect::<Vec<_>>()
                .join(" AND ")
        );
        assert!(parse_join(&excess).is_err());
        assert!(
            parse_join("SELECT * FROM a JOIN b ON a.id = b.id AND a.user_id != b.user_id").is_err()
        );
    }

    #[test]
    fn parses_parenthesized_join_equalities() {
        for statement in [
            "SELECT * FROM a JOIN b ON (a.id = b.id)",
            "SELECT * FROM a JOIN b ON (a.id = b.id AND (a.k = b.k))",
            "SELECT * FROM a JOIN b ON (a.id = b.id) AND (a.k = b.k)",
        ] {
            let parsed = parse_join(statement).unwrap().unwrap();
            assert_eq!(parsed.keys.len(), statement.matches(" = ").count());
        }
        for statement in [
            "SELECT * FROM a JOIN b ON (a.id = b.id",
            "SELECT * FROM a JOIN b ON a.id = b.id)",
            "SELECT * FROM a JOIN b ON (a.id = b.id OR a.k = b.k)",
            "SELECT * FROM a JOIN b ON ()",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
        let deep = format!(
            "SELECT * FROM a JOIN b ON {}a.id = b.id{}",
            "(".repeat(17),
            ")".repeat(17)
        );
        assert!(parse_join(&deep).is_err());
    }

    #[test]
    fn removes_duplicate_and_reversed_join_equalities() {
        let parsed = parse_join(
            "SELECT * FROM a JOIN b ON a.id = b.id AND b.id = a.id AND a.k = b.k AND a.id = b.id",
        )
        .unwrap()
        .unwrap();
        assert_eq!(parsed.keys.len(), 2);
        assert_eq!(parsed.keys[0].0.name, "id");
        assert_eq!(parsed.keys[1].0.name, "k");
    }

    #[test]
    fn join_on_requires_two_known_distinct_relations() {
        for statement in [
            "SELECT * FROM a x JOIN b y ON x.id = x.parent",
            "SELECT * FROM a x JOIN b y ON y.id = y.parent",
            "SELECT * FROM a x JOIN b y ON x.id = z.id",
            "SELECT * FROM a x JOIN b y ON z.id = y.id",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
        assert!(
            parse_join("SELECT * FROM a x JOIN b y ON y.id = x.id")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn parses_using_keys_with_aliases_and_rejects_invalid_lists() {
        let query = parse_join(
            "SELECT l.id, r.name FROM lefts AS l FULL JOIN rights AS r USING (id, region) LIMIT 5",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.kind, JoinType::Full);
        assert_eq!(query.using_keys, ["id", "region"]);
        assert_eq!(query.keys[0].0.relation, "l");
        assert_eq!(query.keys[0].1.relation, "r");
        assert_eq!(query.keys[1].0.name, "region");
        for sql in [
            "SELECT * FROM l JOIN r USING id",
            "SELECT * FROM l JOIN r USING ()",
            "SELECT * FROM l JOIN r USING (id, id)",
            "SELECT * FROM l JOIN r USING (id,) ",
            "SELECT * FROM l JOIN r USING (id) ON l.id = r.id",
        ] {
            assert!(parse_join(sql).is_err(), "{sql}");
        }
        let too_many = format!(
            "SELECT * FROM l JOIN r USING ({})",
            (0..9)
                .map(|n| format!("key{n}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(parse_join(&too_many).is_err());
    }

    #[test]
    fn parses_qualified_wildcard_projections() {
        let query =
            parse_join("SELECT l.*, r.label, r.* FROM lefts l JOIN rights r ON l.id = r.id")
                .unwrap()
                .unwrap();
        assert_eq!(
            query
                .columns
                .unwrap()
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["*", "label", "*"]
        );
        assert!(
            parse_join("SELECT id, r.* FROM lefts l JOIN rights r ON l.id = r.id")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn parses_clickhouse_style_join_limit_offset_count() {
        let query = parse_join("SELECT * FROM l JOIN r ON l.id = r.id LIMIT 7, 3")
            .unwrap()
            .unwrap();
        assert_eq!(query.offset, 7);
        assert_eq!(query.limit, Some(3));
        let query = parse_join("SELECT * FROM l JOIN r ON l.id = r.id LIMIT 3 OFFSET 7")
            .unwrap()
            .unwrap();
        assert_eq!(query.offset, 7);
        assert_eq!(query.limit, Some(3));
        assert!(parse_join("SELECT * FROM l JOIN r ON l.id = r.id LIMIT 2, 3 OFFSET 1").is_err());
        assert!(parse_join("SELECT * FROM l JOIN r ON l.id = r.id LIMIT 2, -1").is_err());
    }

    #[test]
    fn parses_offset_without_limit() {
        let query = parse_join("SELECT * FROM l JOIN r ON l.id = r.id OFFSET 3")
            .unwrap()
            .unwrap();
        assert_eq!(query.limit, None);
        assert_eq!(query.offset, 3);
        let query = parse_join("SELECT * FROM l JOIN r ON l.id = r.id ORDER BY l.id OFFSET 2")
            .unwrap()
            .unwrap();
        assert_eq!(query.offset, 2);
        assert!(parse_join("SELECT * FROM l JOIN r ON l.id = r.id OFFSET -1").is_err());
    }

    #[test]
    fn parses_any_inner_and_left_join_strictness() {
        for statement in [
            "SELECT * FROM a ANY JOIN b ON a.id = b.id",
            "SELECT * FROM a ANY INNER JOIN b ON a.id = b.id",
            "SELECT * FROM a LEFT ANY JOIN b ON a.id = b.id",
            "SELECT * FROM a ANY LEFT JOIN b ON a.id = b.id",
        ] {
            assert!(parse_join(statement).unwrap().unwrap().any, "{statement}");
        }
        for statement in [
            "SELECT * FROM a RIGHT ANY JOIN b ON a.id = b.id",
            "SELECT * FROM a ANY RIGHT JOIN b ON a.id = b.id",
        ] {
            let query = parse_join(statement).unwrap().unwrap();
            assert!(query.any);
            assert_eq!(query.kind, JoinType::Right);
        }
        let ordinary = parse_join("SELECT * FROM a JOIN b ON a.id = b.id")
            .unwrap()
            .unwrap();
        assert!(!ordinary.any);
        let statement = "SELECT * FROM a ANY FULL JOIN b ON a.id = b.id";
        assert!(parse_join(statement).is_err(), "{statement}");
    }

    #[test]
    fn parses_explicit_all_join_strictness() {
        for statement in [
            "SELECT * FROM a ALL JOIN b ON a.id = b.id",
            "SELECT * FROM a ALL INNER JOIN b ON a.id = b.id",
            "SELECT * FROM a INNER ALL JOIN b ON a.id = b.id",
            "SELECT * FROM a LEFT ALL JOIN b USING (id)",
            "SELECT * FROM a RIGHT ALL JOIN b USING (id)",
            "SELECT * FROM a FULL ALL JOIN b USING (id)",
        ] {
            assert!(!parse_join(statement).unwrap().unwrap().any, "{statement}");
        }
        for statement in [
            "SELECT * FROM a ALL ANY JOIN b ON a.id = b.id",
            "SELECT * FROM a ANY INNER ALL JOIN b ON a.id = b.id",
            "SELECT * FROM a ALL INNER ALL JOIN b ON a.id = b.id",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
    }

    #[test]
    fn parses_post_join_null_filters() {
        let query = parse_join(
            "SELECT l.id FROM lefts l LEFT JOIN rights r USING (id) WHERE r.id IS NULL ORDER BY l.id",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.where_null.unwrap().0.relation, "r");
        let query = parse_join(
            "SELECT l.id FROM lefts l JOIN rights r ON l.id = r.id WHERE l.id IS NOT NULL",
        )
        .unwrap()
        .unwrap();
        assert!(query.where_null.unwrap().1);
        let comparison = parse_join("SELECT * FROM l JOIN r ON l.id = r.id WHERE r.id = 1")
            .unwrap()
            .unwrap();
        assert!(comparison.where_comparison.is_some());
        let sql = "SELECT * FROM l JOIN r ON l.id = r.id WHERE r.id IS UNKNOWN";
        assert!(
            parse_join(sql).unwrap().unwrap().where_null.is_some(),
            "{sql}"
        );
    }

    #[test]
    fn parses_explicit_join_null_sort_order() {
        for (suffix, expected) in [
            ("NULLS FIRST", Some(true)),
            ("DESC NULLS LAST", Some(false)),
            ("ASC", None),
        ] {
            let statement =
                format!("SELECT * FROM l LEFT JOIN r ON l.id = r.id ORDER BY r.id {suffix}");
            let query = parse_join(&statement).unwrap().unwrap();
            assert_eq!(query.order_by[0].2, expected);
        }
        assert!(
            parse_join("SELECT * FROM l JOIN r ON l.id = r.id ORDER BY l.id NULLS MIDDLE").is_err()
        );
    }

    #[test]
    fn parses_multiple_join_sort_columns() {
        let query = parse_join(
            "SELECT l.id, r.rank FROM lefts l JOIN rights r USING (id) ORDER BY r.rank DESC NULLS LAST, l.id ASC LIMIT 10",
        )
        .unwrap()
        .unwrap();
        assert_eq!(query.order_by.len(), 2);
        assert_eq!(query.order_by[0].0.relation, "r");
        assert!(query.order_by[0].1);
        assert_eq!(query.order_by[0].2, Some(false));
        assert_eq!(query.order_by[1].0.name, "id");
        assert!(!query.order_by[1].1);
        let too_many = format!(
            "SELECT * FROM l JOIN r USING (id) ORDER BY {}",
            ["l.id"; 65].join(", ")
        );
        assert!(parse_join(&too_many).is_err());
    }

    #[test]
    fn grouped_sum_can_be_used_directly_in_order_by() {
        let query = parse_join(
            "SELECT l.name, SUM(r.amount) FROM lefts l JOIN rights r ON l.id = r.id GROUP BY l.name ORDER BY SUM(r.amount) DESC, l.name",
        )
        .unwrap()
        .unwrap();

        assert_eq!(query.order_by.len(), 2);
        assert!(query.order_by[0].0.is_count_order());
        assert!(query.order_by[0].1);
        assert_eq!(query.order_by[1].0.name, "name");
        assert!(parse_join(
            "SELECT l.name, SUM(r.amount) FROM lefts l JOIN rights r ON l.id = r.id GROUP BY l.name ORDER BY SUM(r.other) DESC"
        )
        .is_err());
    }

    #[test]
    fn parses_global_case_sums_over_a_full_outer_join() {
        let query = parse_join(
            "SELECT SUM(CASE WHEN ssci.customer_sk IS NOT NULL AND csci.customer_sk IS NULL THEN 1 ELSE 0 END) store_only, SUM(CASE WHEN ssci.customer_sk IS NULL AND csci.customer_sk IS NOT NULL THEN 1 ELSE 0 END) catalog_only, SUM(CASE WHEN ssci.customer_sk IS NOT NULL AND csci.customer_sk IS NOT NULL THEN 1 ELSE 0 END) store_and_catalog FROM ssci FULL OUTER JOIN csci ON ssci.customer_sk = csci.customer_sk AND ssci.item_sk = csci.item_sk",
        )
        .unwrap()
        .unwrap();

        assert_eq!(query.kind, JoinType::Full);
        assert_eq!(query.grouped_count, Some(Vec::new()));
        assert_eq!(query.grouped_case_counts.len(), 3);
        assert!(query.grouped_execution_keys.is_empty());
    }

    #[test]
    fn grouped_sum_can_collapse_unprojected_group_keys_in_a_join_chain() {
        let query = parse_join(
            "SELECT c.id, c.name, c.manager, SUM(r.loss) AS total FROM call_center c JOIN returns r ON c.id = r.call_center_id JOIN date_dim d ON r.date_id = d.id GROUP BY c.id, c.name, c.manager, r.marital_status, r.education_status ORDER BY SUM(r.loss) DESC",
        )
        .unwrap()
        .unwrap();

        assert_eq!(query.grouped_count.as_ref().unwrap().len(), 3);
        assert!(query.grouped_sum.is_some());
        assert!(query.order_by[0].0.is_count_order());
    }

    #[test]
    fn scalar_count_accepts_its_noop_order_expression() {
        let query = parse_join(
            "SELECT COUNT(*) FROM sales s JOIN dates d ON s.date_id = d.id ORDER BY COUNT(*) LIMIT 100",
        )
        .unwrap()
        .unwrap();

        assert_eq!(query.aggregate, Some(JoinAggregate::CountStar));
        assert!(query.grouped_count.is_none());
        assert!(query.order_by[0].0.is_count_order());
    }

    #[test]
    fn scalar_sum_accepts_filtered_inner_join_chains() {
        let query = parse_join(
            "SELECT SUM(b.amount) FROM fact f JOIN dim_a a ON f.a_id = a.id JOIN dim_b b ON f.b_id = b.id WHERE b.kind IN ('a', 'b')",
        )
        .unwrap()
        .unwrap();

        assert!(query.scalar_sum.is_some());
        assert_eq!(query.scalar_sum_qualifier.as_deref(), Some("b"));
        assert!(query.aggregate.is_none());
        assert!(query.scalar_where.as_ref().is_some_and(|branches| {
            branches
                .iter()
                .flatten()
                .any(|predicate| matches!(predicate, crate::Predicate::In { .. }))
        }));
    }

    #[test]
    fn parses_multiple_scalar_join_aggregates_with_aliases() {
        let query = parse_join(
            "SELECT AVG(f.quantity) avg_quantity, SUM(f.amount) AS total, COUNT(*) rows, COUNT(f.amount) AS present, COUNT(DISTINCT f.amount) AS unique_amounts FROM fact f JOIN dimension d ON f.id = d.id",
        )
        .unwrap()
        .unwrap();

        assert!(query.aggregate.is_none());
        assert!(query.columns.is_none());
        assert_eq!(query.scalar_aggregates.len(), 5);
        assert_eq!(query.scalar_aggregates[0].alias, "avg_quantity");
        assert_eq!(query.scalar_aggregates[1].alias, "total");
        assert_eq!(query.scalar_aggregates[2].alias, "rows");
        assert_eq!(query.scalar_aggregates[3].alias, "present");
        assert_eq!(query.scalar_aggregates[4].alias, "unique_amounts");
        assert!(query.scalar_aggregates[4].distinct);
        assert_eq!(
            query.scalar_aggregates[2].aggregate,
            JoinAggregate::CountStar
        );
        assert!(
            parse_join("SELECT SUM(f.amount), f.id FROM fact f JOIN dimension d ON f.id = d.id")
                .is_err()
        );
    }

    #[test]
    fn parses_scalar_count_distinct_as_first_metric_and_order_key() {
        let query = parse_join(
            "SELECT COUNT(DISTINCT f.id) AS unique_ids, SUM(f.amount) AS total FROM fact f JOIN dimension d ON f.id = d.id ORDER BY COUNT(DISTINCT f.id) LIMIT 100",
        )
        .unwrap()
        .unwrap();

        assert_eq!(query.scalar_aggregates.len(), 2);
        assert!(query.scalar_aggregates[0].distinct);
        assert_eq!(
            query.scalar_aggregates[0].aggregate,
            JoinAggregate::CountColumn(JoinColumn {
                relation: "f".into(),
                name: "id".into(),
            })
        );
        assert_eq!(query.order_by.len(), 1);
    }

    #[test]
    fn grouped_avg_accepts_implicit_output_aliases() {
        let query = parse_join(
            "SELECT i.id, AVG(s.quantity) avg_quantity, AVG(p.price) avg_price FROM item i JOIN sales s ON i.id = s.item_id JOIN prices p ON i.id = p.item_id GROUP BY i.id ORDER BY i.id",
        )
        .unwrap()
        .unwrap();

        assert_eq!(query.grouped_avg_metrics.len(), 2);
        assert_eq!(query.grouped_avg_metrics[0].1, "avg_quantity");
        assert_eq!(query.grouped_avg_metrics[1].1, "avg_price");
    }

    #[test]
    fn mixed_grouped_metrics_keep_projection_order() {
        let query = parse_join(
            "SELECT a.k, COUNT(*) c1, MIN(a.x) min_x, MAX(b.x) max_x, AVG(c.x) avg_x, b.k, COUNT(*) c2, MIN(b.x), MAX(c.x), AVG(b.x) FROM a JOIN b ON a.k = b.k JOIN c ON b.k = c.k GROUP BY a.k, b.k ORDER BY a.k, b.k",
        )
        .unwrap()
        .unwrap();

        assert_eq!(query.grouped_min_metrics.len(), 2);
        assert_eq!(query.grouped_max_metrics.len(), 2);
        assert_eq!(query.grouped_avg_metrics.len(), 2);
        assert_eq!(
            query.grouped_output,
            vec![
                JoinGroupedOutput::Key(0),
                JoinGroupedOutput::Count("c1".into()),
                JoinGroupedOutput::Min(0, "min_x".into()),
                JoinGroupedOutput::Max(0, "max_x".into()),
                JoinGroupedOutput::Average(0, "avg_x".into()),
                JoinGroupedOutput::Key(1),
                JoinGroupedOutput::Count("c2".into()),
                JoinGroupedOutput::Min(1, "min(b.x)".into()),
                JoinGroupedOutput::Max(1, "max(c.x)".into()),
                JoinGroupedOutput::Average(1, "avg(b.x)".into()),
            ]
        );
    }

    #[test]
    fn rejects_ambiguous_or_unsupported_join_syntax() {
        for sql in [
            "SELECT * FROM users JOIN users ON users.id = users.id",
            "SELECT * FROM a JOIN b ON a.id > b.id",
            "SELECT * FROM a JOIN b ON a.id = b.id OR a.id = b.id",
        ] {
            assert!(parse_join(sql).is_err(), "{sql}");
        }
    }
    #[test]
    fn rejects_unknown_projected_qualifiers_during_parse() {
        for statement in [
            "SELECT c.id FROM lefts a JOIN rights b ON a.id = b.id",
            "SELECT c.* FROM lefts a JOIN rights b ON a.id = b.id LIMIT 0",
        ] {
            assert!(parse_join(statement).is_err(), "{statement}");
        }
        assert!(parse_join("SELECT a.*, b.id FROM lefts a JOIN rights b ON a.id = b.id").is_ok());
    }
}

#[cfg(test)]
mod join_count_one_tests {
    use super::*;

    #[test]
    fn explains_chained_grouped_sum() {
        let query = parse_join("SELECT a.id, SUM(c.amount) AS total FROM a JOIN b ON a.id = b.id JOIN c ON b.id = c.id GROUP BY a.id").unwrap().unwrap();
        let plan = explain_join(&query);
        assert!(plan.contains("GROUP BY a.id with SUM(c.amount)"), "{plan}");
    }

    #[test]
    fn counts_joined_rows_for_literal_one() {
        for aggregate in ["COUNT(*)", "COUNT(1)"] {
            let sql = format!("SELECT {aggregate} FROM l JOIN r ON l.id = r.id");
            let query = parse_join(&sql).unwrap().unwrap();
            assert!(
                matches!(query.aggregate, Some(JoinAggregate::CountStar)),
                "{sql}"
            );
        }
        assert!(parse_join("SELECT COUNT(2) FROM l JOIN r ON l.id = r.id").is_err());
    }
}

#[cfg(test)]
mod join_count_zero_tests {
    use super::*;

    #[test]
    fn zero_constant_counts_all_join_rows() {
        let query = parse_join("SELECT COUNT(0) FROM l JOIN r ON l.id = r.id")
            .unwrap()
            .unwrap();
        assert_eq!(query.aggregate, Some(JoinAggregate::CountStar));
        assert!(parse_join("SELECT COUNT(0, 1) FROM l JOIN r ON l.id = r.id").is_err());
    }
}

#[cfg(test)]
mod nested_implicit_join_tests {
    use super::*;

    #[test]
    fn detects_only_scalar_having_comma_joins() {
        assert!(has_implicit_join("SELECT a.id, SUM(a.v) FROM a JOIN b ON a.id = b.id GROUP BY a.id HAVING SUM(a.v) > (SELECT SUM(v * 1) FROM c, d WHERE c.id = d.id)").unwrap());
        assert!(!has_implicit_join("SELECT a.id, SUM(a.v) FROM a JOIN b ON a.id = b.id GROUP BY a.id HAVING SUM(a.v) > (SELECT SUM(v * 1) FROM c JOIN d ON c.id = d.id)").unwrap());
        assert!(
            !has_implicit_join(
                "SELECT a.id FROM a JOIN b ON a.id = b.id ORDER BY a.id LIMIT (SELECT x FROM c, d)"
            )
            .unwrap()
        );
        assert!(has_implicit_join("SELECT a.id, SUM(a.v) FROM a JOIN b ON a.id = b.id GROUP BY a.id HAVING SUM(a.v) > (SELECT SUM(v * 1) FROM c, d").is_err());
    }
}

#[cfg(test)]
mod scalar_order_projection_tests {
    use super::*;

    #[test]
    fn names_unaliased_rounded_divisions_like_sql_expressions() {
        let query = parse_join(
            "SELECT round(sat_sales1 / sat_sales2, 2) FROM weekly a JOIN weekly b ON a.week = b.week",
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            query.scalar_projections[0].alias,
            "round((sat_sales1 / sat_sales2), 2)"
        );
    }

    #[test]
    fn orders_by_a_selected_substring_expression() {
        let query = parse_join(
            "SELECT a.id, SUBSTRING(a.name, 1, 3) AS short_name FROM a JOIN b ON a.id = b.id ORDER BY SUBSTRING(a.name, 1, 3) DESC",
        )
        .unwrap()
        .unwrap();

        assert_eq!(query.scalar_projections[0].alias, "short_name");
        assert_eq!(query.order_by[0].0.scalar_order_index(), Some(0));
        assert!(query.order_by[0].1);
    }

    #[test]
    fn parses_tpcds_q79_outer_projection_and_ordering() {
        let query = parse_join_with_schema(
            "SELECT c_last_name, c_first_name, SUBSTRING(s_city, 1, 30), ss_ticket_number, amt, profit FROM dn, customer, customer_address current_addr WHERE ss_customer_sk = c_customer_sk AND customer.c_current_addr_sk = current_addr.ca_address_sk AND current_addr.ca_city <> bought_city ORDER BY c_last_name NULLS FIRST, c_first_name NULLS FIRST, SUBSTRING(s_city, 1, 30) NULLS FIRST, profit NULLS FIRST, ss_ticket_number LIMIT 100",
            |table| {
                let columns: &[&str] = match table {
                    "dn" => &[
                        "ss_customer_sk",
                        "ss_ticket_number",
                        "amt",
                        "profit",
                        "bought_city",
                        "s_city",
                    ],
                    "customer" => &["c_customer_sk", "c_current_addr_sk", "c_last_name", "c_first_name"],
                    "customer_address" => &["ca_address_sk", "ca_city"],
                    _ => return Err(ParseError(format!("unknown test table {table}"))),
                };
                Ok(columns.iter().map(|column| (*column).to_owned()).collect())
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(query.scalar_projections.len(), 1);
        assert_eq!(query.order_by[2].0.scalar_order_index(), Some(0));
    }
}

#[cfg(test)]
mod tpcds_derived_query_tests {
    use super::*;

    fn q79_schema(table: &str) -> Result<Vec<DerivedColumn>, ParseError> {
        let names: &[&str] = match table {
            "store_sales" => &[
                "ss_ticket_number",
                "ss_customer_sk",
                "ss_addr_sk",
                "ss_sold_date_sk",
                "ss_store_sk",
                "ss_hdemo_sk",
                "ss_coupon_amt",
                "ss_net_profit",
            ],
            "date_dim" => &["d_date_sk", "d_dow", "d_year"],
            "store" => &["s_store_sk", "s_city", "s_number_employees"],
            "household_demographics" => &["hd_demo_sk", "hd_dep_count", "hd_vehicle_count"],
            "customer" => &[
                "c_customer_sk",
                "c_last_name",
                "c_first_name",
                "c_current_addr_sk",
            ],
            "customer_address" => &["ca_address_sk", "ca_city"],
            _ => return Err(ParseError(format!("unknown test table {table}"))),
        };
        Ok(names
            .iter()
            .map(|name| DerivedColumn {
                name: (*name).into(),
                data_type: "String".into(),
            })
            .collect())
    }

    #[test]
    fn parses_tpcds_q79_derived_aggregate_and_substring_order() {
        let sql = r#"
            SELECT c_last_name, c_first_name, SUBSTRING(s_city, 1, 30),
                   ss_ticket_number, amt, profit
            FROM (
                SELECT ss_ticket_number, ss_customer_sk, store.s_city,
                       SUM(ss_coupon_amt) AS amt, SUM(ss_net_profit) AS profit
                FROM store_sales, date_dim, store, household_demographics
                WHERE store_sales.ss_sold_date_sk = date_dim.d_date_sk
                  AND store_sales.ss_store_sk = store.s_store_sk
                  AND store_sales.ss_hdemo_sk = household_demographics.hd_demo_sk
                  AND (household_demographics.hd_dep_count = 6
                       OR household_demographics.hd_vehicle_count > 2)
                  AND date_dim.d_dow = 1
                  AND date_dim.d_year IN (1999, 1999 + 1, 1999 + 2)
                  AND store.s_number_employees BETWEEN 200 AND 295
                GROUP BY ss_ticket_number, ss_customer_sk, ss_addr_sk, store.s_city
            ) ms, customer
            WHERE ss_customer_sk = c_customer_sk
            ORDER BY c_last_name NULLS FIRST, c_first_name NULLS FIRST,
                     SUBSTRING(s_city, 1, 30) NULLS FIRST, profit NULLS FIRST,
                     ss_ticket_number
            LIMIT 100
        "#;

        let sum = parse_derived_join_sum_with_schema(sql, |table| {
            q79_schema(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
        });
        assert!(matches!(sum, Ok(None)), "{sum:?}");

        let count = parse_derived_join_count_with_schema(sql, |table| {
            q79_schema(table).map(|columns| columns.into_iter().map(|column| column.name).collect())
        });
        assert!(matches!(count, Ok(None)), "{count:?}");

        let grouped = parse_derived_grouped_join_with_schema(sql, q79_schema);
        assert!(grouped.is_ok(), "{grouped:?}");
        let grouped = grouped.unwrap();
        assert!(grouped.is_some());
        let grouped = grouped.unwrap();
        assert_eq!(grouped.outer().scalar_projections.len(), 1);
        assert_eq!(grouped.inner().grouped_execution_keys.len(), 4);
        assert!(
            grouped
                .inner()
                .grouped_execution_keys
                .iter()
                .any(|column| column.name == "ss_addr_sk")
        );
        assert_eq!(
            grouped.outer().scalar_projections[0].alias,
            "\"substring\"(s_city, 1, 30)"
        );
        assert_eq!(grouped.outer().order_by[2].0.scalar_order_index(), Some(0));
    }
}
