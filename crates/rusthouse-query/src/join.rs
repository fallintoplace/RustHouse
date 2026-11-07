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

// Keep explained execution bounds and the executor's actual limits in sync.
pub(crate) const MAX_BUFFERED_JOIN_ROWS: u64 = 32_768;
pub(crate) const MAX_STREAM_JOIN_BUILD_ROWS: u64 = 262_144;
pub(crate) const MAX_JOIN_RESULT_ROWS: usize = 131_072;
pub(crate) const MAX_JOIN_BUILD_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_JOIN_CHAIN_BYTES: usize = 128 * 1024 * 1024;
// Cover JOB's 17-table queries while keeping join chains bounded.
const MAX_JOIN_TABLES: usize = 17;
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

/// Find only explicit JOINs in the outer FROM clause, ignoring nested SELECTs.
/// The parser intentionally scans beyond other clauses for compatibility;
/// the feature probe stops at the end of FROM as before.
fn has_top_level_join_after_from(tokens: &[Token], stop_at_clause: bool) -> bool {
    let mut depth = 0usize;
    let mut seen_from = false;
    for token in tokens {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("FROM") => {
                seen_from = true;
            }
            Token::Word(word)
                if depth == 0 && seen_from && word.eq_ignore_ascii_case("JOIN") =>
            {
                return true;
            }
            Token::Word(word)
                if stop_at_clause
                    && depth == 0
                    && seen_from
                    && ["WHERE", "GROUP", "ORDER", "LIMIT", "OFFSET"]
                        .iter()
                        .any(|clause| word.eq_ignore_ascii_case(clause)) =>
            {
                break;
            }
            _ => {}
        }
    }
    false
}

/// Whether the outer SELECT has an explicit JOIN.
pub fn has_explicit_join(sql: &str) -> Result<bool, ParseError> {
    Ok(has_top_level_join_after_from(&tokenize(sql)?, true))
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

fn parse_join_tokens_internal(
    tokens: Vec<Token>,
    scalar_subquery: bool,
    allow_two_table_grouped_avg: bool,
    allow_ordered_comparison_filters: bool,
) -> Result<Option<JoinSelect>, ParseError> {
    if !matches!(tokens.first(), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT")) {
        return Ok(None);
    }
    // Nested joins do not select the outer JOIN grammar.
    let outer_join = has_top_level_join_after_from(&tokens, false);
    if !outer_join {
        return Ok(None);
    }
    let mut parser = Parser { tokens, index: 0 };
    parser.keyword("SELECT");
    parser.keyword("ALL");
    let mut scalar_sum = None;
    let mut scalar_sum_qualifier = None;
    let mut aggregate_distinct = false;
    let starts_with_grouped_case_sums = matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("SUM"))
        && parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
        && matches!(parser.tokens.get(parser.index + 2), Some(Token::Word(name)) if name.eq_ignore_ascii_case("CASE"));
    let parsed_formula = parse_join_formula(&mut parser)?;
    let scalar_formula = parsed_formula.as_ref().map(|(formula, _)| formula.clone());
    let mut aggregate = if scalar_formula.is_some() {
        None
    } else if starts_with_grouped_case_sums {
        None
    } else if parser.keyword("COUNT") {
        if !parser.take(Token::LParen) {
            return Err(ParseError("JOIN COUNT requires '('".into()));
        }
        aggregate_distinct = parser.keyword("DISTINCT");
        let result = if !aggregate_distinct && parser.take(Token::Star) {
            JoinAggregate::CountStar
        } else if !aggregate_distinct
            && matches!(
                parser.tokens.get(parser.index),
                Some(Token::Number(value)) if value == "0" || value == "1"
            )
        {
            parser.index += 1;
            JoinAggregate::CountStar
        } else {
            let column = projected(&mut parser)?;
            if column.name == "*" {
                return Err(ParseError("JOIN COUNT requires a column".into()));
            }
            JoinAggregate::CountColumn(column)
        };
        if !parser.take(Token::RParen) {
            return Err(ParseError("JOIN COUNT requires ')'".into()));
        }
        Some(result)
    } else {
        let function = if parser.keyword("SUM") {
            Some(crate::AggregateFunction::Sum)
        } else if parser.keyword("AVG") {
            Some(crate::AggregateFunction::Avg)
        } else if parser.keyword("MIN") {
            Some(crate::AggregateFunction::Min)
        } else if parser.keyword("MAX") {
            Some(crate::AggregateFunction::Max)
        } else {
            None
        };
        if let Some(function) = function {
            if !parser.take(Token::LParen) {
                return Err(ParseError("JOIN aggregate requires '('".into()));
            }
            let start = parser.index;
            let column = projected(&mut parser).ok().filter(|column| {
                column.name != "*" && parser.tokens.get(parser.index) == Some(&Token::RParen)
            });
            if let Some(column) = column {
                parser.take(Token::RParen);
                Some(JoinAggregate::Numeric { function, column })
            } else if function == crate::AggregateFunction::Sum {
                parser.index = start;
                let expression = crate::expression::parse_expression_tokens(&mut parser)?;
                if !numeric_join_expression(&expression) || !parser.take(Token::RParen) {
                    return Err(ParseError(
                        "JOIN SUM requires a numeric arithmetic expression".into(),
                    ));
                }
                scalar_sum = Some(expression);
                None
            } else {
                return Err(ParseError("JOIN aggregate requires a column".into()));
            }
        } else {
            None
        }
    };
    if scalar_subquery {
        if let Some(JoinAggregate::Numeric {
            function: crate::AggregateFunction::Sum,
            column,
        }) = &aggregate
        {
            scalar_sum = Some(crate::ScalarExpr::Column(column.name.clone()));
            if !column.relation.is_empty() {
                scalar_sum_qualifier = Some(column.relation.clone());
            }
            aggregate = None;
        }
    }
    let mut aggregate_alias = if let Some((_, alias)) = parsed_formula {
        Some(alias)
    } else if aggregate.is_some() || scalar_sum.is_some() {
        parser.select_output_alias()?
    } else {
        None
    };
    let mut scalar_aggregates = Vec::new();
    if matches!(parser.tokens.get(parser.index), Some(Token::Comma)) {
        let first = aggregate
            .clone()
            .ok_or_else(|| ParseError("multiple JOIN aggregates must use simple columns".into()))?;
        let alias = aggregate_alias
            .take()
            .unwrap_or_else(|| join_scalar_aggregate_header(&first, aggregate_distinct));
        scalar_aggregates.push(JoinScalarAggregate {
            aggregate: first,
            alias,
            distinct: aggregate_distinct,
        });
        while parser.take(Token::Comma) {
            if scalar_aggregates.len() == 16 {
                return Err(ParseError(
                    "JOIN supports at most 16 scalar aggregates".into(),
                ));
            }
            let (next, distinct) = if parser.keyword("COUNT") {
                if !parser.take(Token::LParen) {
                    return Err(ParseError("JOIN COUNT requires '('".into()));
                }
                let distinct = parser.keyword("DISTINCT");
                let aggregate = if !distinct && parser.take(Token::Star) {
                    JoinAggregate::CountStar
                } else {
                    let column = projected(&mut parser)?;
                    if column.name == "*" {
                        return Err(ParseError("JOIN COUNT requires a column".into()));
                    }
                    JoinAggregate::CountColumn(column)
                };
                if !parser.take(Token::RParen) {
                    return Err(ParseError("JOIN COUNT requires ')'".into()));
                }
                (aggregate, distinct)
            } else {
                let function = if parser.keyword("SUM") {
                    Some(crate::AggregateFunction::Sum)
                } else if parser.keyword("AVG") {
                    Some(crate::AggregateFunction::Avg)
                } else if parser.keyword("MIN") {
                    Some(crate::AggregateFunction::Min)
                } else if parser.keyword("MAX") {
                    Some(crate::AggregateFunction::Max)
                } else {
                    None
                }
                .ok_or_else(|| {
                    ParseError("multiple JOIN projections must be scalar aggregates".into())
                })?;
                if !parser.take(Token::LParen) {
                    return Err(ParseError("JOIN aggregate requires '('".into()));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("JOIN aggregate requires one column".into()));
                }
                (JoinAggregate::Numeric { function, column }, false)
            };
            let alias = parser
                .select_output_alias()?
                .unwrap_or_else(|| join_scalar_aggregate_header(&next, distinct));
            scalar_aggregates.push(JoinScalarAggregate {
                aggregate: next,
                alias,
                distinct,
            });
        }
        aggregate = None;
        scalar_sum = None;
        scalar_sum_qualifier = None;
    }
    if aggregate_distinct && scalar_aggregates.is_empty() {
        return Err(ParseError(
            "JOIN COUNT DISTINCT requires a scalar aggregate list".into(),
        ));
    }
    let mut grouped_count_projection = false;
    let mut grouped_count_column = None;
    let mut grouped_distinct = None;
    let mut grouped_case_counts = Vec::new();
    let mut grouped_sum = None;
    let mut grouped_sum_extras = Vec::new();
    let mut grouped_min = None;
    let mut grouped_min_metrics = Vec::new();
    let mut grouped_max_metrics = Vec::new();
    let mut grouped_avg_metrics = Vec::new();
    let mut grouped_window_average = None;
    let mut grouped_rollup = false;
    let mut grouped_formula = None;
    let mut grouped_window_ratio = None;
    let mut grouped_year: Option<(usize, String)> = None;
    let mut grouped_key_aliases: Vec<(usize, String)> = Vec::new();
    let mut grouped_sum_position = None;
    let mut grouped_output_candidate = Vec::new();
    let mut scalar_projections = Vec::new();
    let mut projection_order = Vec::new();
    if starts_with_grouped_case_sums {
        parse_global_case_metrics(&mut parser, &mut grouped_case_counts)?;
        grouped_count_projection = true;
    }
    let columns = if starts_with_grouped_case_sums {
        Some(Vec::new())
    } else if aggregate.is_some()
        || !scalar_aggregates.is_empty()
        || scalar_sum.is_some()
        || scalar_formula.is_some()
        || parser.take(Token::Star)
    {
        None
    } else {
        let mut columns = Vec::new();
        loop {
            if grouped_window_ratio.is_none() {
                if let Some(ratio) = parse_grouped_window_ratio(&mut parser, grouped_sum.as_ref())?
                {
                    grouped_output_candidate
                        .push(JoinGroupedOutput::WindowRatio(ratio.alias.clone()));
                    grouped_window_ratio = Some(ratio);
                    grouped_count_projection = true;
                    break;
                }
            }
            if grouped_sum.is_some()
                && matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("SUM"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                if matches!(parser.tokens.get(parser.index + 2), Some(Token::Word(name)) if name.eq_ignore_ascii_case("CASE"))
                {
                    return Err(ParseError(
                        "JOIN cannot mix grouped SUM and CASE metrics".into(),
                    ));
                }
                if grouped_sum_extras.len() == 8 {
                    return Err(ParseError(
                        "JOIN supports at most 9 grouped SUM metrics".into(),
                    ));
                }
                let (metric, alias) = parse_grouped_sum_projection(&mut parser)?;
                let duplicate_alias = aggregate_alias
                    .as_ref()
                    .is_some_and(|previous| previous.eq_ignore_ascii_case(&alias))
                    || grouped_sum_extras
                        .iter()
                        .any(|(_, previous): &(GroupedSum, String)| {
                            previous.eq_ignore_ascii_case(&alias)
                        })
                    || columns
                        .iter()
                        .any(|column: &JoinColumn| column.name.eq_ignore_ascii_case(&alias))
                    || grouped_key_aliases
                        .iter()
                        .any(|(_, previous)| previous.eq_ignore_ascii_case(&alias));
                if duplicate_alias {
                    return Err(ParseError(
                        "duplicate or ambiguous grouped SUM alias".into(),
                    ));
                }
                grouped_sum_extras.push((metric, alias));
                grouped_output_candidate.push(JoinGroupedOutput::SumExtra(
                    grouped_sum_extras.len() - 1,
                    grouped_sum_extras
                        .last()
                        .expect("grouped SUM just pushed")
                        .1
                        .clone(),
                ));
                grouped_count_projection = true;
                if parser.take(Token::Comma) {
                    continue;
                }
                break;
            }
            if let Some(window_average) = parse_grouped_window_average(&mut parser)? {
                if grouped_window_average.is_some() {
                    return Err(ParseError("JOIN supports one grouped window AVG".into()));
                }
                if grouped_output_alias_exists(
                    &grouped_output_candidate,
                    &grouped_key_aliases,
                    &columns,
                    &window_average.alias,
                ) {
                    return Err(ParseError(
                        "duplicate or ambiguous grouped window AVG alias".into(),
                    ));
                }
                grouped_output_candidate.push(JoinGroupedOutput::WindowAverage(
                    window_average.alias.clone(),
                ));
                grouped_window_average = Some(window_average);
                grouped_count_projection = true;
                if parser.take(Token::Comma) {
                    continue;
                }
                break;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("MIN"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                parser.keyword("MIN");
                if grouped_sum.is_some() || grouped_formula.is_some() {
                    return Err(ParseError("JOIN cannot mix grouped MIN with SUM".into()));
                }
                if grouped_min_metrics.len() == 8 {
                    return Err(ParseError(
                        "JOIN supports at most 8 grouped MIN metrics".into(),
                    ));
                }
                if !parser.take(Token::LParen) {
                    return Err(ParseError("grouped JOIN MIN requires '('".into()));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("grouped JOIN MIN requires one column".into()));
                }
                let alias = parser
                    .select_output_alias()?
                    .unwrap_or_else(|| format!("min({})", column.display_name()));
                if grouped_output_alias_exists(
                    &grouped_output_candidate,
                    &grouped_key_aliases,
                    &columns,
                    &alias,
                ) {
                    return Err(ParseError(
                        "duplicate or ambiguous grouped MIN alias".into(),
                    ));
                }
                let metric_index = grouped_min_metrics.len();
                if grouped_min.is_none() {
                    grouped_min = Some(column.clone());
                    aggregate_alias.get_or_insert_with(|| alias.clone());
                    grouped_sum_position.get_or_insert(columns.len());
                }
                grouped_min_metrics.push((column, alias.clone()));
                grouped_output_candidate.push(JoinGroupedOutput::Min(metric_index, alias));
                grouped_count_projection = true;
                if parser.take(Token::Comma) {
                    continue;
                }
                break;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("MAX"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                parser.keyword("MAX");
                if grouped_sum.is_some() || grouped_formula.is_some() {
                    return Err(ParseError("JOIN cannot mix grouped MAX with SUM".into()));
                }
                if grouped_max_metrics.len() == 8 {
                    return Err(ParseError(
                        "JOIN supports at most 8 grouped MAX metrics".into(),
                    ));
                }
                if !parser.take(Token::LParen) {
                    return Err(ParseError("grouped JOIN MAX requires '('".into()));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("grouped JOIN MAX requires one column".into()));
                }
                let alias = parser
                    .select_output_alias()?
                    .unwrap_or_else(|| format!("max({})", column.display_name()));
                if grouped_output_alias_exists(
                    &grouped_output_candidate,
                    &grouped_key_aliases,
                    &columns,
                    &alias,
                ) {
                    return Err(ParseError(
                        "duplicate or ambiguous grouped MAX alias".into(),
                    ));
                }
                let metric_index = grouped_max_metrics.len();
                grouped_max_metrics.push((column, alias.clone()));
                grouped_output_candidate.push(JoinGroupedOutput::Max(metric_index, alias));
                grouped_count_projection = true;
                if parser.take(Token::Comma) {
                    continue;
                }
                break;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("AVG"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                if grouped_avg_metrics.len() == 8 {
                    return Err(ParseError(
                        "JOIN supports at most 8 grouped AVG metrics".into(),
                    ));
                }
                parser.keyword("AVG");
                if !parser.take(Token::LParen) {
                    return Err(ParseError("grouped JOIN AVG requires '('".into()));
                }
                let column = projected_or_cast(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("grouped JOIN AVG requires one column".into()));
                }
                let alias = parser
                    .select_output_alias()?
                    .unwrap_or_else(|| format!("avg({})", column.display_name()));
                if grouped_output_alias_exists(
                    &grouped_output_candidate,
                    &grouped_key_aliases,
                    &columns,
                    &alias,
                ) {
                    return Err(ParseError(
                        "duplicate or ambiguous grouped AVG alias".into(),
                    ));
                }
                let metric_index = grouped_avg_metrics.len();
                grouped_avg_metrics.push((column, alias.clone()));
                grouped_output_candidate.push(JoinGroupedOutput::Average(metric_index, alias));
                grouped_count_projection = true;
                if parser.take(Token::Comma) {
                    continue;
                }
                break;
            }
            if columns.len() + scalar_projections.len() == 64 {
                return Err(ParseError(
                    "JOIN supports at most 64 projected columns".into(),
                ));
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("EXTRACT"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                parser.keyword("EXTRACT");
                if grouped_year.is_some()
                    || !parser.take(Token::LParen)
                    || !parser.keyword("YEAR")
                    || !parser.keyword("FROM")
                {
                    return Err(ParseError(
                        "JOIN supports one EXTRACT(YEAR FROM column)".into(),
                    ));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) || !parser.keyword("AS") {
                    return Err(ParseError(
                        "JOIN EXTRACT YEAR requires a column and alias".into(),
                    ));
                }
                let alias = parser.output_alias_word()?;
                grouped_year = Some((columns.len(), alias));
                grouped_output_candidate.push(JoinGroupedOutput::Key(columns.len()));
                projection_order.push(JoinProjectionOrder::Column(columns.len()));
                columns.push(column);
            } else if scalar_projection_ahead(&parser) {
                let (encoded, original_ends) =
                    encode_qualified_expression_tokens(&parser.tokens[parser.index..]);
                let mut expression_parser = Parser {
                    tokens: encoded,
                    index: 0,
                };
                let expression =
                    crate::expression::parse_expression_tokens(&mut expression_parser)?;
                if expression_parser.index == 0 {
                    return Err(ParseError(
                        "JOIN scalar projection requires an expression".into(),
                    ));
                }
                let consumed = original_ends
                    .get(expression_parser.index.saturating_sub(1))
                    .copied()
                    .ok_or_else(|| {
                        ParseError("JOIN scalar projection requires an expression".into())
                    })?;
                parser.index += consumed;
                let alias = parser
                    .select_output_alias()?
                    .unwrap_or_else(|| implicit_scalar_projection_alias(&expression));
                let index = scalar_projections.len();
                scalar_projections.push(JoinScalarProjection { expression, alias });
                projection_order.push(JoinProjectionOrder::Scalar(index));
            } else {
                let column = projected(&mut parser)?;
                if let Some(alias) = parser.select_output_alias()? {
                    if column.name == "*" {
                        return Err(ParseError("JOIN group alias requires a column".into()));
                    }
                    grouped_key_aliases.push((columns.len(), alias));
                }
                grouped_output_candidate.push(JoinGroupedOutput::Key(columns.len()));
                projection_order.push(JoinProjectionOrder::Column(columns.len()));
                columns.push(column);
            }
            if !parser.take(Token::Comma) {
                break;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if ["MIN", "MAX", "AVG"].iter().any(|metric| name.eq_ignore_ascii_case(metric)))
            {
                continue;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("AVG"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                loop {
                    if grouped_avg_metrics.len() == 8 {
                        return Err(ParseError(
                            "JOIN supports at most 8 grouped AVG metrics".into(),
                        ));
                    }
                    if !parser.keyword("AVG") || !parser.take(Token::LParen) {
                        return Err(ParseError("grouped JOIN expects AVG(".into()));
                    }
                    let column = projected_or_cast(&mut parser)?;
                    if column.name == "*" || !parser.take(Token::RParen) {
                        return Err(ParseError("grouped JOIN AVG requires one column".into()));
                    }
                    let alias = parser
                        .select_output_alias()?
                        .unwrap_or_else(|| format!("avg({})", column.display_name()));
                    if grouped_avg_metrics
                        .iter()
                        .any(|(_, previous): &(JoinColumn, String)| {
                            previous.eq_ignore_ascii_case(&alias)
                        })
                        || grouped_key_aliases
                            .iter()
                            .any(|(_, key)| key.eq_ignore_ascii_case(&alias))
                    {
                        return Err(ParseError(
                            "duplicate or ambiguous grouped AVG alias".into(),
                        ));
                    }
                    grouped_avg_metrics.push((column, alias));
                    if !parser.take(Token::Comma) {
                        break;
                    }
                    if !matches!(parser.tokens.get(parser.index), Some(Token::Word(name))
                        if name.eq_ignore_ascii_case("AVG"))
                    {
                        return Err(ParseError(
                            "grouped AVG projection expects another AVG".into(),
                        ));
                    }
                }
                grouped_count_projection = true;
                break;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("SUM"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                if let Some((formula, name)) = parse_join_formula(&mut parser)? {
                    if grouped_sum.is_some() || grouped_min.is_some() || grouped_formula.is_some() {
                        return Err(ParseError("JOIN supports one grouped aggregate".into()));
                    }
                    grouped_formula = Some(formula);
                    aggregate_alias = Some(name);
                    grouped_sum_position = Some(columns.len());
                    grouped_count_projection = true;
                    break;
                }
                if grouped_sum.is_some() || grouped_min.is_some() || grouped_formula.is_some() {
                    return Err(ParseError("JOIN supports one grouped aggregate".into()));
                }
                parser.keyword("SUM");
                if !matches!(parser.tokens.get(parser.index + 1), Some(Token::Word(word)) if word.eq_ignore_ascii_case("CASE"))
                {
                    if !parser.take(Token::LParen) {
                        return Err(ParseError("JOIN SUM requires '('".into()));
                    }
                    let start = parser.index;
                    let column = projected(&mut parser).ok().filter(|column| {
                        column.name != "*"
                            && parser.tokens.get(parser.index) == Some(&Token::RParen)
                    });
                    let metric = if let Some(column) = column {
                        GroupedSum::Column(column)
                    } else {
                        parser.index = start;
                        let expression = crate::expression::parse_expression_tokens(&mut parser)?;
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
                        GroupedSum::Expression(expression) => {
                            crate::multi_aggregate::render_expr(expression)
                        }
                    };
                    let alias = parser
                        .select_output_alias()?
                        .unwrap_or_else(|| format!("sum({name})"));
                    grouped_output_candidate.push(JoinGroupedOutput::Sum(alias.clone()));
                    aggregate_alias = Some(alias);
                    grouped_sum = Some(metric);
                    grouped_sum_position = Some(columns.len());
                    grouped_count_projection = true;
                    if parser.take(Token::Comma) {
                        continue;
                    }
                    break;
                }
                loop {
                    if grouped_case_counts.len() >= MAX_JOIN_GROUPED_CASE_METRICS {
                        return Err(ParseError(format!(
                            "JOIN supports at most {MAX_JOIN_GROUPED_CASE_METRICS} grouped CASE metrics"
                        )));
                    }
                    if !parser.take(Token::LParen) {
                        return Err(ParseError("JOIN SUM requires '('".into()));
                    }
                    let (metric, filter) =
                        super::grouped_metric_value(&mut parser, crate::AggregateFunction::Sum)?;
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
                    grouped_case_counts.push((
                        filter.expect("CASE filter checked"),
                        grouped_case_value(metric),
                        parser.output_alias_word()?,
                    ));
                    if !parser.take(Token::Comma) {
                        break;
                    }
                    if parser.keyword("COUNT") {
                        if !parser.take(Token::LParen)
                            || !parser.take(Token::Star)
                            || !parser.take(Token::RParen)
                        {
                            return Err(ParseError(
                                "JOIN CASE metrics can be followed only by COUNT(*)".into(),
                            ));
                        }
                        let alias = parser
                            .select_output_alias()?
                            .unwrap_or_else(|| "count(*)".into());
                        if grouped_case_counts
                            .iter()
                            .any(|(_, _, name)| name.eq_ignore_ascii_case(&alias))
                            || grouped_key_aliases
                                .iter()
                                .any(|(_, name)| name.eq_ignore_ascii_case(&alias))
                        {
                            return Err(ParseError(
                                "duplicate or ambiguous grouped CASE alias".into(),
                            ));
                        }
                        grouped_case_counts.push((
                            crate::GroupedFilter::CaseExpression {
                                condition: crate::ScalarExpr::Compare(
                                    Box::new(crate::ScalarExpr::Integer(1)),
                                    crate::Operator::Eq,
                                    Box::new(crate::ScalarExpr::Integer(1)),
                                ),
                                else_zero: true,
                            },
                            GroupedCaseValue::Count,
                            alias,
                        ));
                        break;
                    }
                    if !parser.keyword("SUM") {
                        return Err(ParseError("JOIN grouped CASE expects SUM metrics".into()));
                    }
                }
                grouped_count_projection = true;
                break;
            }
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("MIN"))
                && matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
            {
                parser.keyword("MIN");
                if grouped_sum.is_some() || grouped_min.is_some() || grouped_formula.is_some() {
                    return Err(ParseError("JOIN supports one grouped aggregate".into()));
                }
                if !parser.take(Token::LParen) {
                    return Err(ParseError("grouped JOIN MIN requires '('".into()));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("grouped JOIN MIN requires one column".into()));
                }
                aggregate_alias = Some(
                    parser
                        .select_output_alias()?
                        .unwrap_or_else(|| format!("min({})", column.display_name())),
                );
                grouped_min = Some(column);
                grouped_sum_position = Some(columns.len());
                grouped_count_projection = true;
                if parser.take(Token::Comma) {
                    continue;
                }
                break;
            }
            if parser.keyword("COUNT") {
                if grouped_sum.is_some() || grouped_formula.is_some() {
                    return Err(ParseError("JOIN supports one grouped aggregate".into()));
                }
                if !parser.take(Token::LParen) {
                    return Err(ParseError("grouped JOIN COUNT requires '('".into()));
                }
                if parser.keyword("DISTINCT") {
                    let column = projected(&mut parser)?;
                    if column.name == "*" || !parser.take(Token::RParen) {
                        return Err(ParseError(
                            "grouped JOIN COUNT DISTINCT requires a column".into(),
                        ));
                    }
                    aggregate_alias = Some(format!("count(DISTINCT {})", column.display_name()));
                    grouped_distinct = Some(column);
                } else if parser.take(Token::Star) {
                    if !parser.take(Token::RParen) {
                        return Err(ParseError("grouped JOIN COUNT requires ')'".into()));
                    }
                    let alias = parser
                        .select_output_alias()?
                        .unwrap_or_else(|| "count(*)".into());
                    if grouped_output_alias_exists(
                        &grouped_output_candidate,
                        &grouped_key_aliases,
                        &columns,
                        &alias,
                    ) {
                        return Err(ParseError(
                            "duplicate or ambiguous grouped COUNT alias".into(),
                        ));
                    }
                    if aggregate_alias.is_none() {
                        aggregate_alias = Some(alias.clone());
                    }
                    grouped_output_candidate.push(JoinGroupedOutput::Count(alias));
                    grouped_count_projection = true;
                    if parser.take(Token::Comma) {
                        continue;
                    }
                    break;
                } else {
                    let column = projected(&mut parser)?;
                    if column.name == "*" || !parser.take(Token::RParen) {
                        return Err(ParseError("grouped JOIN COUNT requires one column".into()));
                    }
                    aggregate_alias = Some(format!("count({})", column.display_name()));
                    grouped_count_column = Some(column);
                }
                grouped_count_projection = true;
                if let Some(alias) = parser.select_output_alias()? {
                    aggregate_alias = Some(alias);
                }
                break;
            }
        }
        Some(columns)
    };
    let grouped_output = if grouped_count_projection
        && grouped_output_candidate
            .iter()
            .any(|item| !matches!(item, JoinGroupedOutput::Key(_)))
    {
        grouped_output_candidate
    } else {
        Vec::new()
    };
    if !parser.keyword("FROM") {
        return Err(ParseError("JOIN requires FROM".into()));
    }
    let left = parser.word()?;
    let left_alias = alias(&mut parser, &left)?;
    if left.len() > 128 || left_alias.len() > 128 {
        return Err(ParseError(
            "table identifiers cannot exceed 128 bytes".into(),
        ));
    }
    let leading_strictness = if parser.keyword("ANY") {
        Some(true)
    } else if parser.keyword("ALL") {
        Some(false)
    } else {
        None
    };
    let cross = parser.keyword("CROSS");
    let kind = if cross {
        JoinType::Inner
    } else {
        parse_join_kind(&mut parser)
    };
    let trailing_strictness = if parser.keyword("ANY") {
        Some(true)
    } else if parser.keyword("ALL") {
        Some(false)
    } else {
        None
    };
    if leading_strictness.is_some() && trailing_strictness.is_some() {
        return Err(ParseError("JOIN strictness specified twice".into()));
    }
    let any = leading_strictness.or(trailing_strictness).unwrap_or(false);
    if cross && (leading_strictness.is_some() || trailing_strictness.is_some()) {
        return Err(ParseError("CROSS JOIN does not accept ANY or ALL".into()));
    }
    if any && !matches!(kind, JoinType::Inner | JoinType::Left | JoinType::Right) {
        return Err(ParseError(
            "ANY JOIN supports only INNER, LEFT and RIGHT joins".into(),
        ));
    }
    if !parser.keyword("JOIN") {
        return Err(ParseError("expected JOIN after join type".into()));
    }
    let right = parser.word()?;
    let right_alias = alias(&mut parser, &right)?;
    if right.len() > 128 || right_alias.len() > 128 {
        return Err(ParseError(
            "table identifiers cannot exceed 128 bytes".into(),
        ));
    }
    if left_alias == right_alias {
        return Err(ParseError("joined tables require distinct aliases".into()));
    }
    let mut on_right_like = None;
    let (keys, using_keys) = if cross {
        (Vec::new(), Vec::new())
    } else if parser.keyword("USING") {
        if !parser.take(Token::LParen) {
            return Err(ParseError("JOIN USING requires '('".into()));
        }
        let mut keys = Vec::new();
        let mut names = Vec::new();
        loop {
            if names.len() >= 8 {
                return Err(ParseError("JOIN USING supports at most 8 keys".into()));
            }
            let name = parser.word()?;
            if names.contains(&name) {
                return Err(ParseError("duplicate JOIN USING column".into()));
            }
            keys.push((
                JoinColumn {
                    relation: left_alias.clone(),
                    name: name.clone(),
                },
                JoinColumn {
                    relation: right_alias.clone(),
                    name: name.clone(),
                },
            ));
            names.push(name);
            if !parser.take(Token::Comma) {
                break;
            }
        }
        if !parser.take(Token::RParen) {
            return Err(ParseError("JOIN USING requires ')'".into()));
        }
        (keys, names)
    } else if parser.keyword("ON") {
        let mut keys = Vec::new();
        equality_group(&mut parser, &mut keys, 0)?;
        if parser.keyword("AND") {
            let column = projected(&mut parser)?;
            let negated = parser.keyword("NOT");
            if !parser.keyword("LIKE") {
                return Err(ParseError(
                    "JOIN ON supports a right-side LIKE filter".into(),
                ));
            }
            let pattern = match parser.tokens.get(parser.index) {
                Some(Token::String(value)) => value.clone(),
                _ => return Err(ParseError("JOIN ON LIKE requires a string literal".into())),
            };
            parser.index += 1;
            on_right_like = Some((column, pattern, negated));
        }
        let mut unique: Vec<(JoinColumn, JoinColumn)> = Vec::with_capacity(keys.len());
        for (first, second) in keys {
            if !unique.iter().any(|(left, right)| {
                (left == &first && right == &second) || (left == &second && right == &first)
            }) {
                unique.push((first, second));
            }
        }
        if unique.len() > 8 {
            return Err(ParseError("JOIN supports at most 8 equality keys".into()));
        }
        (unique, Vec::new())
    } else {
        return Err(ParseError("JOIN requires ON or USING".into()));
    };
    for (first, second) in &keys {
        for column in [first, second] {
            if !column.relation.is_empty()
                && column.relation != left_alias
                && column.relation != right_alias
            {
                return Err(ParseError(format!(
                    "unknown JOIN qualifier: {}",
                    column.relation
                )));
            }
        }
        if !first.relation.is_empty() && first.relation == second.relation {
            return Err(ParseError(
                "JOIN ON must compare one column from each table".into(),
            ));
        }
    }
    let mut steps: Vec<JoinStep> = Vec::new();
    while matches!(
        parser.tokens.get(parser.index),
        Some(Token::Word(word))
            if ["JOIN", "INNER", "LEFT", "RIGHT", "FULL", "CROSS", "ANY", "ALL"]
                .iter()
                .any(|keyword| word.eq_ignore_ascii_case(keyword))
    ) {
        if steps.len() >= MAX_JOIN_TABLES - 2 {
            return Err(ParseError(format!(
                "JOIN supports at most {MAX_JOIN_TABLES} tables"
            )));
        }
        let leading_any = parser.keyword("ANY");
        let leading_all = !leading_any && parser.keyword("ALL");
        let step_cross = parser.keyword("CROSS");
        let step_kind = if step_cross {
            JoinType::Inner
        } else {
            parse_join_kind(&mut parser)
        };
        let trailing_any = parser.keyword("ANY");
        let trailing_all = !trailing_any && parser.keyword("ALL");
        if (leading_any || leading_all) && (trailing_any || trailing_all) {
            return Err(ParseError("JOIN strictness specified twice".into()));
        }
        let step_any = leading_any || trailing_any;
        if step_cross && (leading_any || leading_all || trailing_any || trailing_all) {
            return Err(ParseError("CROSS JOIN does not accept ANY or ALL".into()));
        }
        if !parser.keyword("JOIN") {
            return Err(ParseError("expected JOIN after join type".into()));
        }
        let table = parser.word()?;
        let alias = alias(&mut parser, &table)?;
        if table.len() > 128 || alias.len() > 128 {
            return Err(ParseError(
                "table identifiers cannot exceed 128 bytes".into(),
            ));
        }
        if alias == left_alias
            || alias == right_alias
            || steps.iter().any(|step| step.alias == alias)
        {
            return Err(ParseError("joined tables require distinct aliases".into()));
        }
        if (step_any
            && !matches!(
                step_kind,
                JoinType::Inner | JoinType::Left | JoinType::Right
            ))
            || (any && !matches!(kind, JoinType::Inner | JoinType::Left | JoinType::Right))
        {
            return Err(ParseError(
                "chained ANY JOIN supports only INNER, LEFT and RIGHT joins".into(),
            ));
        }
        let (step_keys, step_using) = if step_cross {
            (Vec::new(), Vec::new())
        } else if parser.keyword("USING") {
            if !parser.take(Token::LParen) {
                return Err(ParseError("JOIN USING requires '('".into()));
            }
            let mut names = Vec::new();
            loop {
                if names.len() >= 8 {
                    return Err(ParseError("JOIN USING supports at most 8 keys".into()));
                }
                let name = parser.word()?;
                if names.contains(&name) {
                    return Err(ParseError("duplicate JOIN USING column".into()));
                }
                names.push(name);
                if !parser.take(Token::Comma) {
                    break;
                }
            }
            if !parser.take(Token::RParen) {
                return Err(ParseError("JOIN USING requires ')'".into()));
            }
            (Vec::new(), names)
        } else if parser.keyword("ON") {
            let mut raw = Vec::new();
            equality_group(&mut parser, &mut raw, 0)?;
            let mut keys: Vec<(JoinColumn, JoinColumn)> = Vec::new();
            for (first, second) in raw {
                for column in [&first, &second] {
                    if !column.relation.is_empty()
                        && column.relation != left_alias
                        && column.relation != right_alias
                        && column.relation != alias
                        && !steps.iter().any(|step| step.alias == column.relation)
                    {
                        return Err(ParseError(format!(
                            "unknown JOIN qualifier: {}",
                            column.relation
                        )));
                    }
                }
                let first_is_new = first.relation == alias;
                let second_is_new = second.relation == alias;
                if !first.relation.is_empty()
                    && !second.relation.is_empty()
                    && first_is_new == second_is_new
                {
                    return Err(ParseError(
                        "JOIN ON must compare an existing relation to the new table".into(),
                    ));
                }
                if !keys.iter().any(|(left, right)| {
                    (left == &first && right == &second) || (left == &second && right == &first)
                }) {
                    keys.push((first, second));
                }
            }
            if keys.len() > 8 {
                return Err(ParseError("JOIN supports at most 8 equality keys".into()));
            }
            (keys, Vec::new())
        } else {
            return Err(ParseError("JOIN requires ON or USING".into()));
        };
        steps.push(JoinStep {
            table,
            alias,
            kind: step_kind,
            any: step_any,
            keys: step_keys,
            using_keys: step_using,
        });
    }
    let mut where_null = None;
    let mut where_comparison = None;
    let mut where_comparisons_extra = Vec::new();
    let mut where_column_comparisons = Vec::new();
    let mut where_substring_comparisons = Vec::new();
    let mut where_in_filters = Vec::new();
    let mut where_like_filters = Vec::new();
    let mut scalar_where = None;
    let mut in_subquery = None;
    if parser.keyword("WHERE") {
        let where_tokens = parser.tokens[parser.index..].iter().take_while(|token| !matches!(token, Token::Word(word) if ["GROUP", "ORDER", "LIMIT", "OFFSET"].iter().any(|name| word.eq_ignore_ascii_case(name)))).collect::<Vec<_>>();
        let boolean_filter = where_tokens
            .iter()
            .any(|token| matches!(token, Token::Word(word) if ["LIKE", "ILIKE", "OR", "BETWEEN", "IN", "CASE"].iter().any(|keyword| word.eq_ignore_ascii_case(keyword))))
            && !where_tokens.iter().any(|token| matches!(token, Token::Word(word) if word.eq_ignore_ascii_case("SELECT")));
        let scalar_expression_filter = where_tokens
            .iter()
            .any(|token| matches!(token, Token::Word(word) if word.eq_ignore_ascii_case("CASE")))
            || {
                let mut depth = 0usize;
                where_tokens.iter().any(|token| match token {
                    Token::LParen => {
                        depth += 1;
                        false
                    }
                    Token::RParen => {
                        depth = depth.saturating_sub(1);
                        false
                    }
                    Token::Plus | Token::Minus | Token::Star | Token::Slash | Token::Percent => {
                        depth == 0
                    }
                    _ => false,
                })
            };
        let complex = scalar_expression_filter
            || grouped_distinct.is_some()
            || boolean_filter
            || where_tokens.iter().any(|token| {
                matches!(token, Token::Word(word) if word.eq_ignore_ascii_case("BETWEEN"))
            })
            || (scalar_sum.is_some()
                || scalar_formula.is_some()
                || !grouped_avg_metrics.is_empty())
                && where_tokens.iter().any(|token| matches!(token, Token::Word(word) if ["OR", "IN", "BETWEEN", "LIKE", "ILIKE", "CASE"].iter().any(|name| word.eq_ignore_ascii_case(name))));
        if grouped_distinct.is_some() {
            if let Some((subquery, branches)) = parse_distinct_not_in(&mut parser)? {
                in_subquery = Some(subquery);
                scalar_where = Some(branches);
            } else {
                scalar_where = Some(crate::where_parser::parse_join_where(&mut parser)?);
            }
        } else if complex {
            scalar_where = Some(crate::where_parser::parse_join_where(&mut parser)?);
        } else {
            let mut count = 0;
            loop {
                count += 1;
                if count > 16 {
                    return Err(ParseError("JOIN supports at most 16 comparisons".into()));
                }
                if matches!(parser.tokens.get(parser.index), Some(Token::Word(name))
                    if name.eq_ignore_ascii_case("SUBSTR")
                        || name.eq_ignore_ascii_case("SUBSTRING"))
                {
                    let (left, start, length) = parse_join_substring(&mut parser)?;
                    let operator = parser.operator()?;
                    if !matches!(
                        operator,
                        crate::Operator::Eq
                            | crate::Operator::Ne
                            | crate::Operator::Lt
                            | crate::Operator::Le
                            | crate::Operator::Gt
                            | crate::Operator::Ge
                    ) {
                        return Err(ParseError(
                            "SUBSTRING requires a comparison operator".into(),
                        ));
                    }
                    let (right, right_start, right_length) = parse_join_substring(&mut parser)?;
                    where_substring_comparisons.push((
                        left,
                        start,
                        length,
                        operator,
                        right,
                        right_start,
                        right_length,
                    ));
                } else {
                    let column = projected(&mut parser)?;
                    if column.name == "*" {
                        return Err(ParseError("JOIN WHERE requires a column".into()));
                    }
                    if parser.keyword("IS") {
                        if count != 1 {
                            return Err(ParseError(
                                "JOIN cannot combine NULL filters with other predicates".into(),
                            ));
                        }
                        let not_null = parser.keyword("NOT");
                        if !parser.keyword("NULL") && !parser.keyword("UNKNOWN") {
                            return Err(ParseError(
                                "JOIN WHERE requires IS [NOT] NULL or UNKNOWN".into(),
                            ));
                        }
                        where_null = Some((column, not_null));
                    } else if parser.keyword("IN") {
                        if !parser.take(Token::LParen) {
                            return Err(ParseError("JOIN IN requires '('".into()));
                        }
                        if matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if word.eq_ignore_ascii_case("SELECT"))
                        {
                            if in_subquery.is_some() || !column.relation.is_empty() {
                                return Err(ParseError(
                                    "JOIN supports one IN subquery with an unqualified key".into(),
                                ));
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
                                return Err(ParseError("JOIN IN subquery requires ')'".into()));
                            }
                            let inner = crate::parse_select_tokens(
                                parser.tokens[start..parser.index - 1].to_vec(),
                            )?;
                            let supported_projection = inner.hidden_aggregate
                                && matches!(
                                    &inner.projection,
                                    crate::Projection::GroupedAggregate {
                                        function: crate::AggregateFunction::Sum,
                                        ..
                                    }
                                )
                                || !inner.hidden_aggregate
                                    && matches!(
                                        &inner.projection,
                                        crate::Projection::Columns(columns) if columns.len() == 1
                                    );
                            if !supported_projection
                                || !matches!(inner.source, Some(crate::Source::Table(_)))
                                || inner
                                    .predicates
                                    .iter()
                                    .chain(inner.or_predicates.iter().flatten())
                                    .any(|predicate| {
                                        matches!(
                                            predicate,
                                            crate::Predicate::ColumnComparison { .. }
                                        )
                                    })
                            {
                                return Err(ParseError(
                                    "JOIN IN requires an uncorrelated grouped SUM subquery".into(),
                                ));
                            }
                            in_subquery = Some(JoinInSubquery {
                                column,
                                inner: Box::new(inner),
                                negated: false,
                            });
                        } else {
                            let mut values = Vec::new();
                            loop {
                                if values.len() == 64 {
                                    return Err(ParseError(
                                        "JOIN IN supports at most 64 values".into(),
                                    ));
                                }
                                values.push(parser.literal()?);
                                if !parser.take(Token::Comma) {
                                    break;
                                }
                            }
                            if !parser.take(Token::RParen) {
                                return Err(ParseError("JOIN IN requires ')'".into()));
                            }
                            where_in_filters.push((column, values));
                        }
                    } else if matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if ["LIKE", "ILIKE", "NOT"].iter().any(|kw| word.eq_ignore_ascii_case(kw)))
                    {
                        let negated = parser.keyword("NOT");
                        let case_insensitive = parser.like_keyword().ok_or_else(|| {
                            ParseError("JOIN WHERE expects LIKE or ILIKE after NOT".into())
                        })?;
                        let pattern = parser.literal()?;
                        where_like_filters.push((column, pattern, case_insensitive, negated));
                    } else {
                        let operator = parser.operator()?;
                        if !matches!(
                            operator,
                            crate::Operator::Eq
                                | crate::Operator::Ne
                                | crate::Operator::Lt
                                | crate::Operator::Le
                                | crate::Operator::Gt
                                | crate::Operator::Ge
                        ) {
                            return Err(ParseError(
                                "JOIN WHERE requires a comparison operator".into(),
                            ));
                        }
                        let is_column = match parser.tokens.get(parser.index) {
                            Some(Token::QuotedWord(_)) => true,
                            Some(Token::Word(word)) => ![
                                "CAST",
                                "DATE",
                                "TIMESTAMP",
                                "NULL",
                                "TRUE",
                                "FALSE",
                                "INTERVAL",
                            ]
                            .iter()
                            .any(|kw| word.eq_ignore_ascii_case(kw)),
                            _ => false,
                        };
                        if is_column {
                            let other = projected(&mut parser)?;
                            if other.name == "*" {
                                return Err(ParseError("JOIN WHERE requires a column".into()));
                            }
                            where_column_comparisons.push((column, operator, other));
                        } else {
                            let comparison = (column, operator, parser.literal()?);
                            if where_comparison.is_none() {
                                where_comparison = Some(comparison);
                            } else {
                                where_comparisons_extra.push(comparison);
                            }
                        }
                    }
                }
                if !parser.keyword("AND") {
                    break;
                }
                if where_null.is_some() {
                    return Err(ParseError(
                        "JOIN cannot combine NULL filters with other predicates".into(),
                    ));
                }
            }
        }
    }
    let group_by = if parser.keyword("GROUP") {
        if !parser.keyword("BY") {
            return Err(ParseError("JOIN GROUP requires BY".into()));
        }
        grouped_rollup = parser.keyword("ROLLUP");
        if grouped_rollup && !parser.take(Token::LParen) {
            return Err(ParseError("ROLLUP requires '('".into()));
        }
        let mut keys = vec![projected(&mut parser)?];
        while parser.take(Token::Comma) {
            if keys.len() >= 64 {
                return Err(ParseError("grouped JOIN supports at most 64 keys".into()));
            }
            keys.push(projected(&mut parser)?);
        }
        if grouped_rollup && !parser.take(Token::RParen) {
            return Err(ParseError("ROLLUP requires ')'".into()));
        }
        for key in &mut keys {
            if key.relation.is_empty() {
                if let Some((position, _)) = grouped_key_aliases
                    .iter()
                    .find(|(_, alias)| key.name.eq_ignore_ascii_case(alias))
                {
                    *key = columns.as_ref().expect("alias projection")[*position].clone();
                }
            }
        }
        if let Some((position, alias)) = &grouped_year {
            let source = &columns.as_ref().expect("year projection")[*position];
            for key in &mut keys {
                if key.relation.is_empty() && key.name.eq_ignore_ascii_case(alias) {
                    *key = source.clone();
                } else if key == source {
                    return Err(ParseError(
                        "GROUP BY must use the EXTRACT YEAR alias".into(),
                    ));
                }
            }
        }
        if keys.iter().any(|key| key.name == "*") {
            return Err(ParseError("JOIN GROUP BY requires columns".into()));
        }
        Some(keys)
    } else {
        None
    };
    let grouped_keys_only = !grouped_count_projection && group_by.is_some();
    if grouped_keys_only {
        if kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || aggregate.is_some()
            || columns.as_ref().is_none_or(Vec::is_empty)
            || grouped_year.is_some()
        {
            return Err(ParseError(
                "GROUP BY without aggregates requires projected keys over INNER joins".into(),
            ));
        }
        grouped_count_projection = true;
    }
    let mut grouped_count_having = None;
    let grouped_sum_having = if parser.keyword("HAVING") {
        if grouped_sum.is_none() && grouped_count_projection {
            if parser.keyword("COUNT") {
                if !parser.take(Token::LParen)
                    || !parser.take(Token::Star)
                    || !parser.take(Token::RParen)
                {
                    return Err(ParseError("HAVING COUNT requires count(*)".into()));
                }
            } else {
                let alias = parser.word()?;
                if !aggregate_alias
                    .as_ref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(&alias))
                {
                    return Err(ParseError("JOIN HAVING must use the COUNT alias".into()));
                }
            }
            let operator = parser.operator()?;
            let value = parser.number()?;
            let valid = if value.contains(['.', 'e', 'E']) {
                value.parse::<f64>().is_ok_and(f64::is_finite)
            } else if value.starts_with('-') {
                value.parse::<i128>().is_ok()
            } else {
                value.parse::<u128>().is_ok()
            };
            if !valid {
                return Err(ParseError("invalid JOIN HAVING number".into()));
            }
            grouped_count_having = Some(GroupedCountHaving { operator, value });
            None
        } else {
            let metric = grouped_sum
                .as_ref()
                .ok_or_else(|| ParseError("JOIN HAVING requires a grouped SUM".into()))?;
            if parser.keyword("SUM") {
                if !parser.take(Token::LParen) {
                    return Err(ParseError("HAVING SUM requires '('".into()));
                }
                let start = parser.index;
                let column = projected(&mut parser).ok().filter(|column| {
                    column.name != "*" && parser.tokens.get(parser.index) == Some(&Token::RParen)
                });
                let having_metric = if let Some(column) = column {
                    GroupedSum::Column(column)
                } else {
                    parser.index = start;
                    GroupedSum::Expression(crate::expression::parse_expression_tokens(&mut parser)?)
                };
                if !parser.take(Token::RParen) || &having_metric != metric {
                    return Err(ParseError("HAVING SUM must match the selected SUM".into()));
                }
            } else {
                let name = parser.word()?;
                if !aggregate_alias
                    .as_ref()
                    .is_some_and(|alias| alias.eq_ignore_ascii_case(&name))
                {
                    return Err(ParseError("JOIN HAVING must use the SUM alias".into()));
                }
                if columns.as_ref().is_some_and(|columns| {
                    columns
                        .iter()
                        .any(|column| column.name.eq_ignore_ascii_case(&name))
                }) {
                    return Err(ParseError(
                        "JOIN HAVING SUM alias conflicts with a group key".into(),
                    ));
                }
            }
            let operator = parser.operator()?;
            let value = parser.number()?;
            let valid = if value.contains(['.', 'e', 'E']) {
                value.parse::<f64>().is_ok_and(f64::is_finite)
            } else if value.starts_with('-') {
                value.parse::<i128>().is_ok()
            } else {
                value.parse::<u128>().is_ok()
            };
            if !valid {
                return Err(ParseError("invalid JOIN HAVING number".into()));
            }
            Some(GroupedSumHaving {
                operator,
                value,
                subquery: None,
            })
        }
    } else {
        None
    };
    let order_by = if parser.keyword("ORDER") {
        if !parser.keyword("BY") {
            return Err(ParseError("ORDER requires BY".into()));
        }
        let mut terms = Vec::new();
        loop {
            if terms.len() == 64 {
                return Err(ParseError("JOIN ORDER BY supports at most 64 keys".into()));
            }
            let next_is_count_alias = grouped_count_projection
                && grouped_case_counts.is_empty()
                && matches!(parser.tokens.get(parser.index), Some(Token::Word(name) | Token::QuotedAlias(name))
                    if name.eq_ignore_ascii_case(aggregate_alias.as_deref().unwrap_or("count")))
                && !matches!(
                    parser.tokens.get(parser.index + 1),
                    Some(Token::Dot | Token::LParen)
                );
            let next_case_alias = (!steps.is_empty())
                .then(|| {
                    let Some(Token::Word(name) | Token::QuotedAlias(name)) =
                        parser.tokens.get(parser.index)
                    else {
                        return None;
                    };
                    grouped_case_counts
                        .iter()
                        .position(|(_, _, alias)| alias.eq_ignore_ascii_case(name))
                })
                .flatten();
            let next_avg_alias = grouped_avg_metrics
                .iter()
                .position(|(_, alias)| {
                    matches!(parser.tokens.get(parser.index), Some(Token::Word(name) | Token::QuotedAlias(name))
                        if name.eq_ignore_ascii_case(alias))
                });
            let next_is_window_ratio_alias = grouped_window_ratio
                .as_ref()
                .is_some_and(|ratio| {
                    matches!(parser.tokens.get(parser.index), Some(Token::Word(name) | Token::QuotedAlias(name))
                        if name.eq_ignore_ascii_case(&ratio.alias))
                });
            let next_is_grouped_sum = grouped_sum.is_some()
                && matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("SUM"))
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen);
            let next_is_scalar_sum = (scalar_sum.is_some()
                || matches!(
                    aggregate.as_ref(),
                    Some(JoinAggregate::Numeric {
                        function: crate::AggregateFunction::Sum,
                        ..
                    })
                ))
                && matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("SUM"))
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen);
            let next_scalar_expression_order = scalar_projection_ahead(&parser).then(|| {
                let (encoded, original_ends) =
                    encode_qualified_expression_tokens(&parser.tokens[parser.index..]);
                let mut expression_parser = Parser {
                    tokens: encoded,
                    index: 0,
                };
                let expression =
                    crate::expression::parse_expression_tokens(&mut expression_parser).ok()?;
                if expression_parser.index == 0 {
                    return None;
                }
                let scalar_index = scalar_projections
                    .iter()
                    .position(|projection| projection.expression == expression)?;
                let consumed = original_ends.get(expression_parser.index - 1).copied()?;
                Some((scalar_index, consumed))
            });
            let next_scalar_expression_order = next_scalar_expression_order.flatten();
            let next_scalar_alias_order = scalar_projections.iter().position(|projection| {
                matches!(parser.tokens.get(parser.index), Some(Token::Word(name) | Token::QuotedAlias(name))
                    if name.eq_ignore_ascii_case(&projection.alias))
                    && !matches!(parser.tokens.get(parser.index + 1), Some(Token::Dot | Token::LParen))
            });
            let scalar_count_distinct_order =
                !scalar_aggregates.is_empty() && parser.keyword("COUNT");
            let key = if scalar_count_distinct_order {
                if !parser.take(Token::LParen) {
                    return Err(ParseError("scalar JOIN ORDER BY COUNT requires '('".into()));
                }
                let distinct = parser.keyword("DISTINCT");
                let column = if parser.take(Token::Star) {
                    None
                } else {
                    Some(projected(&mut parser)?)
                };
                if !parser.take(Token::RParen)
                    || !scalar_aggregates.iter().any(|item| {
                        match (&item.aggregate, item.distinct, &column, distinct) {
                            (JoinAggregate::CountStar, false, None, false) => true,
                            (JoinAggregate::CountColumn(expected), false, Some(actual), false)
                            | (JoinAggregate::CountColumn(expected), true, Some(actual), true) => {
                                expected == actual
                            }
                            _ => false,
                        }
                    })
                {
                    return Err(ParseError(
                        "scalar JOIN ORDER BY COUNT must match a selected metric".into(),
                    ));
                }
                JoinColumn::count_order()
            } else if let Some(index) = next_case_alias {
                parser.index += 1;
                JoinColumn::case_order(index)
            } else if let Some(index) = next_avg_alias {
                parser.index += 1;
                JoinColumn::avg_order(index)
            } else if grouped_rollup && parser.keyword("GROUPING") {
                if !parser.take(Token::LParen) {
                    return Err(ParseError("GROUPING requires '('".into()));
                }
                let key = projected(&mut parser)?;
                if !parser.take(Token::RParen) {
                    return Err(ParseError("GROUPING requires ')'".into()));
                }
                let index = columns
                    .as_ref()
                    .and_then(|keys| keys.iter().position(|candidate| candidate == &key))
                    .ok_or_else(|| ParseError("GROUPING requires a projected rollup key".into()))?;
                JoinColumn::grouping_order(index)
            } else if next_is_window_ratio_alias {
                parser.index += 1;
                JoinColumn::window_ratio_order()
            } else if let Some(index) = next_scalar_alias_order {
                parser.index += 1;
                JoinColumn::scalar_order(index)
            } else if next_is_count_alias {
                parser.index += 1;
                JoinColumn::count_order()
            } else if next_is_grouped_sum {
                parser.keyword("SUM");
                if !parser.take(Token::LParen) {
                    return Err(ParseError("ORDER BY SUM requires '('".into()));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("ORDER BY SUM requires one column".into()));
                }
                if grouped_sum.as_ref() != Some(&GroupedSum::Column(column)) {
                    return Err(ParseError(
                        "ORDER BY SUM must match the selected grouped SUM".into(),
                    ));
                }
                JoinColumn::count_order()
            } else if next_is_scalar_sum {
                parser.keyword("SUM");
                if !parser.take(Token::LParen) {
                    return Err(ParseError("ORDER BY SUM requires '('".into()));
                }
                let column = projected(&mut parser)?;
                if column.name == "*" || !parser.take(Token::RParen) {
                    return Err(ParseError("ORDER BY SUM requires one column".into()));
                }
                let selected_column = match (&scalar_sum, &aggregate) {
                    (Some(crate::ScalarExpr::Column(name)), _) => {
                        Some((name.as_str(), scalar_sum_qualifier.as_deref()))
                    }
                    (
                        _,
                        Some(JoinAggregate::Numeric {
                            function: crate::AggregateFunction::Sum,
                            column,
                        }),
                    ) => Some((column.name.as_str(), Some(column.relation.as_str()))),
                    _ => None,
                };
                if selected_column.is_none_or(|(name, qualifier)| {
                    name != column.name
                        || qualifier.is_some_and(|qualifier| qualifier != column.relation)
                }) {
                    return Err(ParseError(
                        "ORDER BY SUM must match the selected scalar SUM".into(),
                    ));
                }
                JoinColumn::count_order()
            } else if grouped_count_projection
                && grouped_case_counts.is_empty()
                && grouped_sum.is_none()
                && grouped_min.is_none()
                && grouped_formula.is_none()
                && grouped_distinct.is_none()
                && grouped_count_column.is_none()
                && parser.keyword("COUNT")
            {
                if !parser.take(Token::LParen)
                    || !parser.take(Token::Star)
                    || !parser.take(Token::RParen)
                {
                    return Err(ParseError(
                        "grouped JOIN ORDER BY COUNT requires COUNT(*)".into(),
                    ));
                }
                JoinColumn::count_order()
            } else if aggregate == Some(JoinAggregate::CountStar) && parser.keyword("COUNT") {
                if !parser.take(Token::LParen)
                    || !parser.take(Token::Star)
                    || !parser.take(Token::RParen)
                {
                    return Err(ParseError(
                        "scalar JOIN ORDER BY COUNT requires COUNT(*)".into(),
                    ));
                }
                // A scalar COUNT returns one row, so this cannot change its order.
                JoinColumn::count_order()
            } else if grouped_count_projection
                && matches!(parser.tokens.get(parser.index), Some(Token::Number(_)))
            {
                let position = parser
                    .number()?
                    .parse::<usize>()
                    .map_err(|_| ParseError("JOIN ORDER BY position is too large".into()))?;
                if position == 0 {
                    return Err(ParseError("JOIN ORDER BY positions start at 1".into()));
                }
                columns
                    .as_ref()
                    .and_then(|columns| columns.get(position - 1))
                    .cloned()
                    .ok_or_else(|| ParseError("JOIN ORDER BY position is out of range".into()))?
            } else if aggregate.is_none()
                && !grouped_count_projection
                && grouped_sum.is_none()
                && grouped_case_counts.is_empty()
                && !grouped_rollup
                && matches!(parser.tokens.get(parser.index), Some(Token::Number(_)))
            {
                let position = parser
                    .number()?
                    .parse::<usize>()
                    .map_err(|_| ParseError("JOIN ORDER BY position is too large".into()))?;
                if position == 0 {
                    return Err(ParseError("JOIN ORDER BY positions start at 1".into()));
                }
                match projection_order.get(position - 1) {
                    Some(JoinProjectionOrder::Column(index)) => columns
                        .as_ref()
                        .and_then(|columns| columns.get(*index))
                        .cloned()
                        .ok_or_else(|| {
                            ParseError("JOIN ORDER BY position is out of range".into())
                        })?,
                    Some(JoinProjectionOrder::Scalar(index)) => JoinColumn::scalar_order(*index),
                    None => {
                        return Err(ParseError("JOIN ORDER BY position is out of range".into()));
                    }
                }
            } else if let Some((index, consumed)) = next_scalar_expression_order {
                parser.index += consumed;
                JoinColumn::scalar_order(index)
            } else {
                let mut key = projected(&mut parser)?;
                if key.name == "*" {
                    return Err(ParseError("JOIN ORDER BY requires a column".into()));
                }
                if key.relation.is_empty() {
                    if let Some((position, _)) = grouped_key_aliases
                        .iter()
                        .find(|(_, alias)| key.name.eq_ignore_ascii_case(alias))
                    {
                        key = columns.as_ref().expect("alias projection")[*position].clone();
                    }
                }
                if let Some((position, alias)) = &grouped_year {
                    let source = &columns.as_ref().expect("year projection")[*position];
                    if key.relation.is_empty() && key.name.eq_ignore_ascii_case(alias) {
                        key = source.clone();
                    } else if &key == source {
                        return Err(ParseError(
                            "ORDER BY must use the EXTRACT YEAR alias".into(),
                        ));
                    }
                }
                key
            };
            let descending = if parser.keyword("USING") {
                match parser.operator()? {
                    crate::Operator::Lt => false,
                    crate::Operator::Gt => true,
                    _ => return Err(ParseError("JOIN ORDER BY USING expects '<' or '>'".into())),
                }
            } else if parser.keyword("DESC") {
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
                    return Err(ParseError(
                        "JOIN ORDER BY NULLS requires FIRST or LAST".into(),
                    ));
                }
            } else {
                None
            };
            terms.push((key, descending, nulls_first));
            if !parser.take(Token::Comma) {
                break;
            }
        }
        terms
    } else {
        Vec::new()
    };
    let known_qualifier = |relation: &str| {
        relation == left_alias
            || relation == right_alias
            || steps.iter().any(|step| step.alias == relation)
    };
    if let Some(column) = &grouped_count_column {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    if let Some((column, _, _)) = &on_right_like {
        if !column.relation.is_empty() && column.relation != right_alias {
            return Err(ParseError("JOIN ON LIKE must use the right input".into()));
        }
    }
    if let Some(JoinAggregate::CountColumn(column) | JoinAggregate::Numeric { column, .. }) =
        &aggregate
    {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    for item in &scalar_aggregates {
        if let JoinAggregate::CountColumn(column) | JoinAggregate::Numeric { column, .. } =
            &item.aggregate
        {
            if !column.relation.is_empty() && !known_qualifier(&column.relation) {
                return Err(ParseError(format!(
                    "unknown JOIN qualifier: {}",
                    column.relation
                )));
            }
        }
    }
    if let Some(columns) = &columns {
        for column in columns {
            if !column.relation.is_empty() && !known_qualifier(&column.relation) {
                return Err(ParseError(format!(
                    "unknown JOIN qualifier: {}",
                    column.relation
                )));
            }
        }
    }
    if let Some((column, _, _)) = &where_comparison {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    for (column, _, _) in &where_comparisons_extra {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    if let Some(column) = &grouped_distinct {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    if let Some(column) = &grouped_min {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    for (column, _) in grouped_min_metrics.iter().chain(&grouped_max_metrics) {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    if let Some(GroupedSum::Column(column)) = &grouped_sum {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    for (left, _, right) in &where_column_comparisons {
        for column in [left, right] {
            if !column.relation.is_empty() && !known_qualifier(&column.relation) {
                return Err(ParseError(format!(
                    "unknown JOIN qualifier: {}",
                    column.relation
                )));
            }
        }
    }
    for (column, _) in &where_in_filters {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    for (column, _, _, _) in &where_like_filters {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    if let Some((column, _)) = &where_null {
        if !column.relation.is_empty() && !known_qualifier(&column.relation) {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    if let Some(columns) = &group_by {
        for column in columns {
            if !column.relation.is_empty() && !known_qualifier(&column.relation) {
                return Err(ParseError(format!(
                    "unknown JOIN qualifier: {}",
                    column.relation
                )));
            }
        }
    }
    for (column, _, _) in &order_by {
        if !column.is_count_order()
            && !column.relation.is_empty()
            && !known_qualifier(&column.relation)
        {
            return Err(ParseError(format!(
                "unknown JOIN qualifier: {}",
                column.relation
            )));
        }
    }
    let (limit, comma_offset) = if parser.keyword("LIMIT") {
        let first = bound(&mut parser, "LIMIT")?;
        if parser.take(Token::Comma) {
            (Some(bound(&mut parser, "LIMIT")?), Some(first))
        } else {
            (Some(first), None)
        }
    } else {
        (None, None)
    };
    let offset = if let Some(offset) = comma_offset {
        offset
    } else if parser.keyword("OFFSET") {
        bound(&mut parser, "OFFSET")?
    } else {
        0
    };
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError(
            "unexpected JOIN syntax; supported: equi-JOIN, ORDER BY, LIMIT, OFFSET".into(),
        ));
    }
    if !steps.is_empty()
        && kind == JoinType::Inner
        && steps.iter().all(|step| step.kind == JoinType::Inner)
        && group_by.is_none()
        && order_by.is_empty()
        && offset == 0
        && scalar_aggregates.is_empty()
        && matches!(
            aggregate.as_ref(),
            Some(JoinAggregate::Numeric {
                function: crate::AggregateFunction::Min | crate::AggregateFunction::Max,
                ..
            })
        )
    {
        let aggregate = aggregate
            .take()
            .expect("matching scalar extrema aggregate is present");
        let alias = aggregate_alias
            .take()
            .unwrap_or_else(|| join_aggregate_header(&aggregate));
        scalar_aggregates.push(JoinScalarAggregate {
            aggregate,
            alias,
            distinct: false,
        });
    }
    if !where_substring_comparisons.is_empty()
        && (steps.is_empty()
            || kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || aggregate.is_some()
            || !scalar_aggregates.is_empty()
            || scalar_sum.is_some()
            || scalar_formula.is_some()
            || grouped_min.is_some()
            || !grouped_max_metrics.is_empty()
            || grouped_formula.is_some()
            || grouped_distinct.is_some()
            || !grouped_case_counts.is_empty()
            || (!order_by.is_empty() && !grouped_count_projection && grouped_sum.is_none()))
    {
        return Err(ParseError(
            "SUBSTRING comparisons require a streaming INNER join chain".into(),
        ));
    }
    // A chained scalar SUM uses the existing bounded streaming accumulator.
    let mut chained_scalar_sum = false;
    if !steps.is_empty()
        && kind == JoinType::Inner
        && steps.iter().all(|step| step.kind == JoinType::Inner)
        && scalar_sum.is_none()
        && scalar_formula.is_none()
    {
        if let Some(JoinAggregate::Numeric {
            function: crate::AggregateFunction::Sum,
            column,
        }) = &aggregate
        {
            scalar_sum = Some(crate::ScalarExpr::Column(column.name.clone()));
            scalar_sum_qualifier = (!column.relation.is_empty()).then(|| column.relation.clone());
            aggregate = None;
            chained_scalar_sum = true;
        }
    }
    if (scalar_sum.is_some() || scalar_formula.is_some())
        && (kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || (!order_by.is_empty()
                && !(scalar_sum.is_some()
                    && order_by
                        .iter()
                        .all(|(column, _, _)| column.is_count_order())))
            || (!chained_scalar_sum
                && (!where_in_filters.is_empty() || !where_like_filters.is_empty())))
    {
        return Err(ParseError(
            "JOIN expression SUM requires INNER joins and comparison filters".into(),
        ));
    }
    if !scalar_aggregates.is_empty()
        && (kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || group_by.is_some()
            || order_by
                .iter()
                .any(|(column, _, _)| !column.is_count_order())
            || offset > 0)
    {
        return Err(ParseError(
            "multiple scalar JOIN aggregates require an unordered INNER join chain".into(),
        ));
    }
    let chained_comparison_filters = (!steps.is_empty()
        || grouped_sum.is_some()
        || grouped_min.is_some()
        || !grouped_max_metrics.is_empty()
        || grouped_formula.is_some())
        && kind == JoinType::Inner
        && steps.iter().all(|step| step.kind == JoinType::Inner)
        && matches!(
            aggregate.as_ref(),
            None | Some(JoinAggregate::CountStar | JoinAggregate::CountColumn(_))
        )
        && (scalar_sum.is_some() || (where_in_filters.is_empty() && where_like_filters.is_empty()));
    let derived_ordered_comparison_filters = allow_ordered_comparison_filters
        && kind == JoinType::Inner
        && steps.iter().all(|step| step.kind == JoinType::Inner)
        && !grouped_count_projection
        && aggregate.is_none()
        && group_by.is_none()
        && scalar_where.is_none()
        && where_comparison.is_some()
        && where_column_comparisons.is_empty()
        && where_in_filters.is_empty()
        && where_like_filters.is_empty();
    if grouped_case_counts.is_empty()
        && (where_comparison.is_some()
            || !where_column_comparisons.is_empty()
            || !where_in_filters.is_empty()
            || !where_like_filters.is_empty())
        && !chained_comparison_filters
        && !derived_ordered_comparison_filters
        && (kind != JoinType::Inner
            || !steps.is_empty()
            || (grouped_count_projection
                && (columns.as_ref().is_none_or(|keys| keys.len() != 1)
                    || where_comparison.as_ref().is_some_and(|(column, _, _)| {
                        !column.relation.is_empty() && column.relation != left_alias
                    })))
            || (!grouped_count_projection
                && (!order_by.is_empty()
                    || !matches!(aggregate.as_ref(), None | Some(JoinAggregate::CountStar))
                    || group_by.is_some())))
    {
        return Err(ParseError(
            "JOIN comparison WHERE requires an unordered two-table INNER JOIN, or grouped COUNT(*) with a fact-side filter"
                .into(),
        ));
    }
    if grouped_count_projection
        && grouped_case_counts.is_empty()
        && !chained_comparison_filters
        && (!where_comparisons_extra.is_empty()
            || !where_column_comparisons.is_empty()
            || !where_in_filters.is_empty()
            || !where_like_filters.is_empty())
    {
        return Err(ParseError(
            "grouped JOIN supports one comparison filter".into(),
        ));
    }
    if grouped_case_counts
        .iter()
        .any(|(_, value, _)| !matches!(value, GroupedCaseValue::Count))
        && steps.is_empty()
        && kind != JoinType::Inner
    {
        return Err(ParseError(
            "conditional numeric SUM requires an INNER join chain".into(),
        ));
    }
    let global_case_aggregate = grouped_count_projection
        && !grouped_case_counts.is_empty()
        && columns.as_ref().is_some_and(Vec::is_empty)
        && group_by.is_none();
    if !grouped_case_counts.is_empty()
        && !global_case_aggregate
        && (!matches!(kind, JoinType::Inner | JoinType::Left)
            || steps
                .iter()
                .any(|step| !matches!(step.kind, JoinType::Inner | JoinType::Left))
            || where_null.is_some()
            || columns.as_ref().is_none_or(Vec::is_empty))
    {
        return Err(ParseError(
            "JOIN grouped CASE requires group keys over INNER or LEFT inputs".into(),
        ));
    }
    if !grouped_avg_metrics.is_empty()
        && ((!allow_two_table_grouped_avg && steps.is_empty())
            || kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || grouped_sum.is_some()
            || grouped_formula.is_some()
            || grouped_distinct.is_some()
            || grouped_count_column.is_some()
            || !grouped_case_counts.is_empty()
            || order_by
                .iter()
                .any(|(column, _, _)| column.is_count_order()))
    {
        return Err(ParseError(
            "multiple grouped AVGs require an INNER join chain with key ordering".into(),
        ));
    }
    if grouped_distinct.is_some()
        && (kind != JoinType::Inner || steps.iter().any(|step| step.kind != JoinType::Inner))
    {
        return Err(ParseError(
            "grouped COUNT DISTINCT requires INNER joins".into(),
        ));
    }
    if grouped_count_column.is_some()
        && (!steps.is_empty()
            || !matches!(kind, JoinType::Inner | JoinType::Left)
            || columns.as_ref().is_none_or(|columns| columns.len() != 1))
    {
        return Err(ParseError(
            "grouped COUNT(column) requires one key and two INNER or LEFT inputs".into(),
        ));
    }
    if on_right_like.is_some()
        && (kind != JoinType::Left || grouped_count_column.is_none() || !steps.is_empty())
    {
        return Err(ParseError(
            "JOIN ON LIKE requires grouped LEFT JOIN COUNT(column)".into(),
        ));
    }
    let grouped_sum_outer_chain = grouped_sum.is_some()
        && grouped_count_projection
        && columns.as_ref().is_some_and(|keys| !keys.is_empty())
        && group_by.is_some()
        && matches!(kind, JoinType::Inner | JoinType::Left)
        && steps
            .iter()
            .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left));
    let grouped_case_inner_chain = !grouped_case_counts.is_empty()
        && grouped_count_projection
        && columns.as_ref().is_some_and(|keys| !keys.is_empty())
        && group_by.is_some()
        && kind == JoinType::Inner
        && steps.iter().all(|step| step.kind == JoinType::Inner);
    if (grouped_min.is_some()
        || !grouped_max_metrics.is_empty()
        || grouped_formula.is_some()
        || (grouped_sum.is_some() && !grouped_sum_outer_chain))
        && (kind != JoinType::Inner || steps.iter().any(|step| step.kind != JoinType::Inner))
    {
        return Err(ParseError(
            "grouped SUM requires a keyed INNER or LEFT join chain; grouped MIN/MAX requires INNER joins"
                .into(),
        ));
    }
    if !grouped_key_aliases.is_empty() {
        let simple_projection = !grouped_count_projection
            && aggregate.is_none()
            && scalar_aggregates.is_empty()
            && scalar_sum.is_none()
            && scalar_formula.is_none()
            && grouped_sum.is_none()
            && grouped_min.is_none()
            && !grouped_keys_only
            && group_by.is_none();
        let simple_projection_outer_chain = simple_projection
            && matches!(kind, JoinType::Inner | JoinType::Left)
            && steps
                .iter()
                .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left));
        if !(grouped_count_projection || simple_projection)
            || (!(grouped_sum_outer_chain || simple_projection_outer_chain)
                && (kind != JoinType::Inner
                    || steps.iter().any(|step| step.kind != JoinType::Inner)))
            || grouped_count_column.is_some()
            || (!grouped_case_counts.is_empty() && !grouped_case_inner_chain)
        {
            return Err(ParseError(
                "JOIN key aliases require a simple or grouped INNER join".into(),
            ));
        }
        for (index, (_, alias)) in grouped_key_aliases.iter().enumerate() {
            if grouped_key_aliases[..index]
                .iter()
                .any(|(_, other)| other.eq_ignore_ascii_case(alias))
                || grouped_year
                    .as_ref()
                    .is_some_and(|(_, other)| other.eq_ignore_ascii_case(alias))
                || (grouped_count_projection
                    && aggregate_alias
                        .as_deref()
                        .unwrap_or("count")
                        .eq_ignore_ascii_case(alias))
            {
                return Err(ParseError("ambiguous grouped JOIN output alias".into()));
            }
        }
    }
    if let Some((position, alias)) = &grouped_year {
        let keys = columns.as_ref().expect("year projection");
        if !grouped_count_projection
            || kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || keys.iter().enumerate().any(|(index, key)| {
                index != *position
                    && (key == &keys[*position] || key.name.eq_ignore_ascii_case(alias))
            })
            || aggregate_alias
                .as_deref()
                .unwrap_or("count")
                .eq_ignore_ascii_case(alias)
            || !grouped_case_counts.is_empty()
            || grouped_count_column.is_some()
        {
            return Err(ParseError(
                "EXTRACT YEAR requires an unambiguous grouped INNER join".into(),
            ));
        }
    }
    if let Some(ratio) = &grouped_window_ratio {
        let valid_multiplier = ratio.multiplier.parse::<f64>().is_ok_and(f64::is_finite);
        let Some(GroupedSum::Column(_)) = grouped_sum.as_ref() else {
            return Err(ParseError(
                "grouped window ratio requires a selected column SUM".into(),
            ));
        };
        let Some(keys) = columns.as_ref() else {
            return Err(ParseError(
                "grouped window ratio requires projected group keys".into(),
            ));
        };
        if !valid_multiplier
            || !grouped_count_projection
            || steps.is_empty()
            || kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || !keys.contains(&ratio.partition)
            || !group_by
                .as_ref()
                .is_some_and(|groups| groups.contains(&ratio.partition))
            || aggregate_alias
                .as_ref()
                .is_some_and(|alias| alias.eq_ignore_ascii_case(&ratio.alias))
            || keys
                .iter()
                .any(|key| key.name.eq_ignore_ascii_case(&ratio.alias))
            || grouped_key_aliases
                .iter()
                .any(|(_, alias)| alias.eq_ignore_ascii_case(&ratio.alias))
            || (!ratio.partition.relation.is_empty() && !known_qualifier(&ratio.partition.relation))
            || grouped_sum_position.is_none()
            || !grouped_case_counts.is_empty()
            || !grouped_sum_extras.is_empty()
            || grouped_formula.is_some()
            || !grouped_avg_metrics.is_empty()
        {
            return Err(ParseError(
                "grouped window ratio requires a grouped INNER SUM and a projected partition key"
                    .into(),
            ));
        }
    }
    if let Some(window_average) = &grouped_window_average {
        let Some(GroupedSum::Column(sum_column)) = grouped_sum.as_ref() else {
            return Err(ParseError(
                "grouped window AVG requires a selected column SUM".into(),
            ));
        };
        let Some(groups) = group_by.as_ref() else {
            return Err(ParseError("grouped window AVG requires GROUP BY".into()));
        };
        let Some(keys) = columns.as_ref() else {
            return Err(ParseError(
                "grouped window AVG requires projected group keys".into(),
            ));
        };
        if grouped_window_ratio.is_some()
            || grouped_sum_extras.len() > 0
            || !grouped_case_counts.is_empty()
            || !grouped_min_metrics.is_empty()
            || !grouped_max_metrics.is_empty()
            || !grouped_avg_metrics.is_empty()
            || grouped_formula.is_some()
            || grouped_rollup
            || steps.is_empty()
            || kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || &window_average.value != sum_column
            || window_average.partition.is_empty()
            || window_average
                .partition
                .iter()
                .any(|key| !groups.contains(key) || !keys.contains(key))
            || window_average
                .partition
                .iter()
                .enumerate()
                .any(|(index, key)| window_average.partition[..index].contains(key))
            || keys
                .iter()
                .any(|key| key.name.eq_ignore_ascii_case(&window_average.alias))
            || grouped_key_aliases
                .iter()
                .any(|(_, alias)| alias.eq_ignore_ascii_case(&window_average.alias))
            || aggregate_alias
                .as_ref()
                .is_some_and(|alias| alias.eq_ignore_ascii_case(&window_average.alias))
        {
            return Err(ParseError(
                "grouped window AVG requires a grouped INNER SUM and projected partition key"
                    .into(),
            ));
        }
    }
    let groups_include_unprojected_keys = grouped_sum.is_some()
        && !steps.is_empty()
        && group_by.as_ref().is_some_and(|groups| {
            columns
                .as_ref()
                .is_some_and(|projected| groups.iter().any(|key| !projected.contains(key)))
        });
    let grouped_execution_keys =
        if grouped_window_average.is_some() || groups_include_unprojected_keys {
            group_by.clone().unwrap_or_default()
        } else {
            Vec::new()
        };
    let grouped_count = if grouped_count_projection {
        let projected_keys = columns
            .as_ref()
            .ok_or_else(|| ParseError("grouped JOIN requires projected keys".into()))?;
        let matching_groups = global_case_aggregate
            || if !steps.is_empty() {
                group_by.as_ref().is_some_and(|groups| {
                    groups == projected_keys
                        || (groups.len() == projected_keys.len()
                            && groups.iter().all(|key| projected_keys.contains(key))
                            && groups
                                .iter()
                                .enumerate()
                                .all(|(index, key)| !groups[..index].contains(key))
                            && projected_keys
                                .iter()
                                .enumerate()
                                .all(|(index, key)| !projected_keys[..index].contains(key)))
                })
            } else {
                group_by.as_ref() == Some(projected_keys)
            };
        let grouped_sum_can_collapse_unprojected_keys = !steps.is_empty()
            && grouped_sum.is_some()
            && group_by
                .as_ref()
                .is_some_and(|groups| projected_keys.iter().all(|key| groups.contains(key)));
        if !matching_groups && !grouped_sum_can_collapse_unprojected_keys {
            return Err(ParseError(
                "GROUP BY must match the selected JOIN keys".into(),
            ));
        }
        if grouped_rollup
            && (grouped_avg_metrics.is_empty()
                || grouped_sum.is_some()
                || !grouped_sum_extras.is_empty()
                || grouped_min.is_some()
                || !grouped_max_metrics.is_empty()
                || grouped_formula.is_some()
                || grouped_window_ratio.is_some()
                || !grouped_case_counts.is_empty()
                || grouped_sum_having.is_some()
                || grouped_count_having.is_some())
        {
            return Err(ParseError(
                "JOIN ROLLUP currently requires grouped AVG metrics without HAVING".into(),
            ));
        }
        let grouped_case_outer_join = !grouped_case_counts.is_empty()
            && ((matches!(kind, JoinType::Inner | JoinType::Left)
                && steps
                    .iter()
                    .all(|step| matches!(step.kind, JoinType::Inner | JoinType::Left)))
                || (global_case_aggregate && kind == JoinType::Full && steps.is_empty()));
        if (!matches!(kind, JoinType::Inner | JoinType::Left | JoinType::Full)
            || (kind == JoinType::Full && !grouped_case_outer_join))
            || (!(grouped_case_outer_join || grouped_sum_outer_chain)
                && ((projected_keys.len() > 1 && kind != JoinType::Inner)
                    || (!steps.is_empty()
                        && (kind != JoinType::Inner
                            || steps.iter().any(|step| step.kind != JoinType::Inner)))))
        {
            return Err(ParseError(
                "chained grouped JOIN COUNT requires INNER joins, except for grouped CASE with LEFT joins".into(),
            ));
        }
        if order_by.iter().any(|(column, _, _)| {
            (column.is_count_order() && grouped_keys_only)
                || (!column.is_count_order()
                    && !column.is_window_ratio_order()
                    && column.case_order_index().is_none()
                    && column.avg_order_index().is_none()
                    && column.grouping_order_index().is_none()
                    && !projected_keys.contains(column))
        }) {
            return Err(ParseError(
                "grouped JOIN ORDER BY must use a projected group key or aggregate".into(),
            ));
        }
        if global_case_aggregate {
            Some(Vec::new())
        } else if steps.is_empty() {
            group_by
        } else {
            Some(projected_keys.clone())
        }
    } else {
        if group_by.is_some() {
            return Err(ParseError(
                "JOIN GROUP BY requires projected COUNT(*)".into(),
            ));
        }
        None
    };
    if let Some(subquery) = &in_subquery {
        let supported = if subquery.negated {
            grouped_distinct.is_some() && scalar_where.is_some()
        } else {
            !steps.is_empty()
                && grouped_sum.is_some()
                && scalar_where.is_none()
                && grouped_count_projection
        };
        if !supported
            || kind != JoinType::Inner
            || steps.iter().any(|step| step.kind != JoinType::Inner)
            || where_null.is_some()
        {
            return Err(ParseError(
                "JOIN subquery requires a supported grouped INNER join".into(),
            ));
        }
    }
    // COUNT(column) can use bounded materialization for an all-INNER chain.
    if matches!(aggregate.as_ref(), Some(JoinAggregate::CountColumn(_)))
        && (kind != JoinType::Inner || steps.iter().any(|step| step.kind != JoinType::Inner))
    {
        return Err(ParseError("JOIN COUNT(column) requires INNER joins".into()));
    }
    // Chained numeric/column aggregates and OUTER or filtered chained COUNT
    // lack a matching streaming semantics implementation. Reject them.
    // COUNT(*) can also use the bounded materializing executor for OUTER
    // chains. Only the all-INNER case uses the multiplicity-only fast path.
    if !steps.is_empty()
        && aggregate.is_some()
        && !matches!(
            aggregate.as_ref(),
            Some(JoinAggregate::CountStar | JoinAggregate::CountColumn(_))
        )
    {
        return Err(ParseError(
            "chained JOIN supports COUNT as its only scalar aggregate".into(),
        ));
    }
    let scalar_count_order = aggregate == Some(JoinAggregate::CountStar)
        && order_by
            .iter()
            .all(|(column, _, _)| column.is_count_order());
    if aggregate.is_some()
        && ((!order_by.is_empty() && !scalar_count_order)
            || (steps.is_empty()
                && kind != JoinType::Inner
                && !matches!(aggregate.as_ref(), Some(JoinAggregate::CountStar))))
    {
        return Err(ParseError(
            "JOIN aggregates require no ORDER BY; two-table OUTER JOIN supports COUNT(*) only"
                .into(),
        ));
    }
    if !scalar_projections.is_empty() {
        let projected_columns = columns
            .as_ref()
            .ok_or_else(|| ParseError("JOIN scalar projections require row output".into()))?;
        let grouped_case_literals = grouped_case_inner_chain
            && scalar_projections.iter().all(|projection| {
                matches!(projection.expression, crate::ScalarExpr::TextLiteral(_))
            });
        if (grouped_count_projection && !grouped_case_literals)
            || aggregate.is_some()
            || !scalar_aggregates.is_empty()
            || scalar_sum.is_some()
            || scalar_formula.is_some()
            || projection_order.len() != projected_columns.len() + scalar_projections.len()
        {
            return Err(ParseError(
                "JOIN scalar projections require a simple row-level SELECT".into(),
            ));
        }
    }
    Ok(Some(JoinSelect {
        left,
        right,
        left_alias,
        right_alias,
        kind,
        any,
        columns,
        scalar_projections,
        projection_order,
        aggregate,
        scalar_aggregates,
        aggregate_alias,
        grouped_count,
        grouped_execution_keys,
        grouped_output,
        grouped_keys_only,
        grouped_count_column,
        on_right_like,
        grouped_distinct,
        grouped_case_counts,
        grouped_sum,
        grouped_sum_extras,
        grouped_min,
        grouped_min_metrics,
        grouped_max_metrics,
        grouped_avg_metrics,
        grouped_window_average,
        grouped_rollup,
        grouped_formula,
        grouped_window_ratio,
        grouped_year,
        grouped_key_aliases,
        grouped_sum_position,
        grouped_sum_having,
        grouped_count_having,
        scalar_sum,
        scalar_sum_qualifier,
        scalar_result_typed: false,
        scalar_formula,
        scalar_where,
        in_subquery,
        keys,
        using_keys,
        where_null,
        where_comparison,
        where_comparisons_extra,
        where_column_comparisons,
        where_substring_comparisons,
        where_in_filters,
        where_like_filters,
        order_by,
        limit,
        offset,
        steps,
        virtual_columns: Vec::new(),
    }))
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
    fn explicit_join_probe_ignores_nested_selects_and_end_of_from() {
        let nested = tokenize("SELECT * FROM (SELECT a FROM t JOIN u ON t.id = u.id) AS x")
            .unwrap();
        assert!(!has_top_level_join_after_from(&nested, true));

        let outer = tokenize("SELECT * FROM t JOIN u ON t.id = u.id").unwrap();
        assert!(has_top_level_join_after_from(&outer, true));
        assert!(has_top_level_join_after_from(&outer, false));

        let after_where = tokenize("SELECT * FROM t WHERE x = 1 JOIN u").unwrap();
        assert!(!has_top_level_join_after_from(&after_where, true));
        assert!(has_top_level_join_after_from(&after_where, false));
    }

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
