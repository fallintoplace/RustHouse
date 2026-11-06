mod copy;
mod correlated_exists;
mod cte_correlated_avg;
mod cte_join;
mod cte_max;
mod date_scalar;
mod ddl;
mod delete;
mod execute;
mod explain;
mod expression;
mod grouped_affine;
mod grouped_case;
mod grouped_regex;
pub use grouped_regex::GroupedRegex;
mod grouped_extract;
pub use grouped_case::GroupedCaseKey;
mod grouped_minute;
pub use grouped_affine::GroupedAffineKey;
pub use grouped_extract::GroupedExtractMinute;
pub use grouped_minute::GroupedMinute;
mod insert;
mod insert_select;
mod join;
mod json;
pub mod memory_budget;
pub use json::parse_json_each_row_fields;
mod bounded_except;
mod lexer;
mod literal;
mod multi_aggregate;
mod scalar;
mod select_parser;
use select_parser::parse_select_tokens;
mod text_scalar;
mod update;
mod where_parser;
pub use bounded_except::{
    DerivedSetCount, DerivedSetOperation, ExceptSelect, execute_bounded_except,
    execute_derived_set_count, parse_bounded_except, parse_derived_set_count_with_schema,
};
pub use copy::{CopyFormat, CopyFrom, parse_copy_from};
pub use correlated_exists::{CorrelatedExists, parse_correlated_exists};
pub use cte_correlated_avg::{
    CteCorrelatedAvg, CteOutputColumn, parse_cte_correlated_avg_with_schema,
};
pub use cte_join::{
    CteMultiJoin, CteMultiJoinSource, CteSourceQuery, parse_cte_multi_join_with_schema,
    parse_cte_union_select_with_schema,
};
pub use cte_max::{CteMaxJoin, has_cte_max_join, parse_cte_max_join};
pub use ddl::{ColumnDef, Ddl, parse_ddl};
pub use delete::parse_delete;
pub use execute::{
    ExecutionError, RowPredicate, bind_join_having_subquery, bind_join_in_subquery, execute,
    execute_correlated_avg, execute_correlated_avg_with_paths, execute_correlated_join,
    execute_correlated_min, execute_correlated_sum, execute_cte_max_join,
    execute_derived_grouped_join, execute_derived_grouped_select, execute_derived_join_count,
    execute_derived_union_join, execute_derived_union_select, execute_join, execute_multi_join,
    execute_nested_join_sum, execute_substring_antijoin, merge_uniq_states,
};
pub use explain::explain_select;
pub use expression::{ScalarExpr, parse_scalar_expression};
pub use insert::{Insert, parse_insert};
pub use insert_select::{InsertSelect, parse_insert_select};
pub(crate) use join::JoinExistsLookupPair;
pub use join::{
    DerivedColumn, DerivedGroupedJoin, DerivedGroupedSelect, DerivedJoinCount, DerivedUnionJoin,
    DerivedUnionSelect, DerivedUnionSelectBranch, GroupedCaseValue, GroupedSum, GroupedWindowRatio,
    JoinAggregate, JoinColumn, JoinCorrelatedAvg, JoinCorrelatedExists, JoinCorrelatedMin,
    JoinCorrelatedSum, JoinExistsDateFilter, JoinExistsFilter, JoinExistsKey, JoinExistsLookup,
    JoinGroupedOutput, JoinProjectionOrder, JoinScalarAggregate, JoinScalarProjection, JoinSelect,
    JoinStep, JoinType, JoinVirtualColumn, NestedJoinSum, SubstringAntiJoin, explain_join,
    has_derived_join_count, has_derived_union, has_explicit_join, has_implicit_join,
    has_join_correlated_avg, has_join_correlated_min, has_join_correlated_sum, has_nested_join_sum,
    has_substring_antijoin, has_top_level_exists, parse_derived_grouped_join_with_schema,
    parse_derived_grouped_select_with_schema, parse_derived_join_count_with_schema,
    parse_derived_join_sum_with_schema, parse_derived_union_join_with_schema,
    parse_derived_union_select_with_schema, parse_join, parse_join_correlated_avg_with_schema,
    parse_join_correlated_exists_with_schema, parse_join_correlated_min_with_schema,
    parse_join_correlated_sum_with_schema, parse_join_with_schema,
    parse_nested_join_sum_with_schema, parse_substring_antijoin_with_schema,
};
pub use multi_aggregate::AggregateItem;
pub use update::{Update, UpdateValue, parse_update};

use std::error::Error;
use std::fmt;

use lexer::{Token, tokenize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateFunction {
    GroupBitXor,
    GroupBitOr,
    GroupBitAnd,
    AnyLast,
    Any,
    CountIf,
    Count,
    CountDistinct,
    Sum,
    Product,
    Avg,
    SumSquares,
    Min,
    Range,
    Max,
    Rms,
    VarPop,
    StddevPop,
    VarSamp,
    StddevSamp,
    Median,
}

impl AggregateFunction {
    pub fn name(self) -> &'static str {
        match self {
            Self::GroupBitXor => "groupBitXor",
            Self::GroupBitOr => "groupBitOr",
            Self::GroupBitAnd => "groupBitAnd",
            Self::AnyLast => "anyLast",
            Self::Any => "any",
            Self::CountIf => "countIf",
            Self::Count => "count",
            Self::CountDistinct => "countDistinct",
            Self::Sum => "sum",
            Self::Product => "product",
            Self::Avg => "avg",
            Self::SumSquares => "sumSquares",
            Self::Min => "min",
            Self::Range => "range",
            Self::Max => "max",
            Self::Rms => "rms",
            Self::VarPop => "varPop",
            Self::StddevPop => "stddevPop",
            Self::VarSamp => "varSamp",
            Self::StddevSamp => "stddevSamp",
            Self::Median => "median",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstantValue {
    Integer(i64),
    /// Floating numeric literal. Keep its source spelling for SELECT headers.
    Decimal(String),
    Text(String),
    Null,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constant {
    pub value: ConstantValue,
    pub alias: Option<String>,
}

impl Constant {
    fn header(&self) -> String {
        if let Some(alias) = &self.alias {
            return alias.clone();
        }
        match &self.value {
            ConstantValue::Integer(value) => value.to_string(),
            ConstantValue::Decimal(value) => value.clone(),
            ConstantValue::Null => "NULL".into(),
            ConstantValue::Text(value) => format!(
                "'{}'",
                value
                    .replace('\\', "\\\\")
                    .replace('\t', "\\t")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\'', "''")
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupedFilter {
    Truthy(String),
    Comparison {
        column: String,
        operator: Operator,
        literal: String,
    },
    CaseElseZero {
        column: String,
        operator: Operator,
        literal: String,
    },
    /// Conditional CASE metric for a NULL or non-NULL input column.
    CaseNull {
        column: String,
        negated: bool,
    },
    /// Multiple comparisons of the same column in a CASE ... THEN 1 ELSE 0.
    CaseElseZeroCombined {
        column: String,
        comparisons: Vec<(Operator, String)>,
        all: bool,
    },
    /// Searched CASE condition that uses a numeric expression across columns.
    CaseExpression {
        condition: ScalarExpr,
        else_zero: bool,
    },
}

impl GroupedFilter {
    pub(crate) fn column(&self) -> Option<&str> {
        match self {
            Self::Truthy(column)
            | Self::Comparison { column, .. }
            | Self::CaseElseZero { column, .. }
            | Self::CaseNull { column, .. }
            | Self::CaseElseZeroCombined { column, .. } => Some(column),
            Self::CaseExpression { .. } => None,
        }
    }

    pub(crate) fn label(&self) -> String {
        match self {
            Self::Truthy(column) => column.clone(),
            Self::CaseExpression { condition, .. } => {
                crate::multi_aggregate::render_expr(condition)
            }
            Self::CaseElseZeroCombined {
                column,
                comparisons,
                all,
            } => {
                let separator = if *all { " AND " } else { " OR " };
                comparisons
                    .iter()
                    .map(|(operator, literal)| {
                        let symbol = match operator {
                            Operator::Eq => "=",
                            Operator::Ne => "!=",
                            Operator::Lt => "<",
                            Operator::Le => "<=",
                            Operator::Gt => ">",
                            Operator::Ge => ">=",
                            _ => "?",
                        };
                        format!("{column} {symbol} {literal}")
                    })
                    .collect::<Vec<_>>()
                    .join(separator)
            }
            Self::Comparison {
                column,
                operator,
                literal,
            }
            | Self::CaseElseZero {
                column,
                operator,
                literal,
            } => {
                let symbol = match operator {
                    Operator::Eq => "=",
                    Operator::Ne => "!=",
                    Operator::Lt => "<",
                    Operator::Le => "<=",
                    Operator::Gt => ">",
                    Operator::Ge => ">=",
                    _ => "?",
                };
                format!("{column} {symbol} {literal}")
            }
            Self::CaseNull { column, negated } => {
                format!("{column} IS {}NULL", if *negated { "NOT " } else { "" })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Projection {
    Literal(i64),
    Constants(Vec<Constant>),
    Count,
    CountColumn {
        column: String,
    },
    CountDistinct {
        column: String,
    },
    /// Fixed-memory approximate cardinality (~0.81% RSE).
    ApproxDistinct {
        column: String,
    },
    /// Bounded-memory histogram with a fixed number of numeric bins.
    Histogram {
        column: String,
        bins: usize,
    },
    /// Stable hex-encoded HyperLogLog registers for partial global reductions.
    ApproxDistinctState {
        column: String,
    },
    GroupedCount {
        column: String,
    },
    GroupedDistinctCount {
        column: String,
        value_column: String,
    },
    GroupedMultiDistinctCount {
        columns: Vec<String>,
        value_column: String,
    },
    GroupedMultiCount {
        columns: Vec<String>,
    },
    GroupedAggregate {
        column: String,
        function: AggregateFunction,
        value_column: String,
    },
    GroupedAggregates {
        column: String,
        items: Vec<(AggregateFunction, String)>,
        aliases: Vec<Option<String>>,
        filters: Vec<Option<GroupedFilter>>,
    },
    GroupedMultiAggregates {
        columns: Vec<String>,
        items: Vec<(AggregateFunction, String)>,
        aliases: Vec<Option<String>>,
        filters: Vec<Option<GroupedFilter>>,
    },
    Aggregate {
        function: AggregateFunction,
        column: String,
    },
    MultipleAggregates(Vec<AggregateItem>),
    StringLength {
        column: String,
    },
    All,
    Columns(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Modulo(u64),
    ModuloNe(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate {
    /// Compare two columns from the same SQL input row.
    ColumnComparison {
        column: String,
        operator: Operator,
        other: String,
    },
    Comparison {
        column: String,
        operator: Operator,
        value: String,
    },
    In {
        column: String,
        values: Vec<String>,
    },
    NotIn {
        column: String,
        values: Vec<String>,
    },
    SubstringIn {
        column: String,
        start: usize,
        length: usize,
        values: Vec<String>,
        has_null: bool,
        negated: bool,
    },
    NotBetween {
        column: String,
        lower: String,
        upper: String,
    },
    IsNull {
        column: String,
        negated: bool,
    },
    /// SQL NULL-safe equality / inequality against a non-NULL literal.
    DistinctFrom {
        column: String,
        value: String,
        negated: bool,
    },
    Like {
        column: String,
        pattern: String,
        case_insensitive: bool,
        negated: bool,
    },
    /// A bounded numeric scalar expression used as a JOIN row predicate.
    Expression(ScalarExpr),
}

impl Predicate {
    fn other_column(&self) -> Option<&str> {
        match self {
            Self::ColumnComparison { other, .. } => Some(other),
            _ => None,
        }
    }

    fn column(&self) -> &str {
        match self {
            Self::ColumnComparison { column, .. }
            | Self::Comparison { column, .. }
            | Self::In { column, .. }
            | Self::NotIn { column, .. }
            | Self::SubstringIn { column, .. }
            | Self::NotBetween { column, .. }
            | Self::IsNull { column, .. }
            | Self::DistinctFrom { column, .. }
            | Self::Like { column, .. } => column,
            Self::Expression(_) => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Table(String),
    Numbers(u64),
    NumbersRange { start: u64, count: u64 },
    NumbersStepped { start: u64, length: u64, step: u64 },
    GenerateSeries { start: u64, count: u64, step: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderBy {
    pub column: String,
    pub descending: bool,
    /// None keeps the existing ASC/DESC NULL ordering.
    pub nulls_first: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Having {
    pub operator: Operator,
    pub value: u64,
    pub aggregate_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Select {
    pub projection: Projection,
    /// An integer literal projected before the grouped source columns.
    pub grouped_literal: Option<i64>,
    /// Extra grouping expressions based on the first integer key.
    pub grouped_affine: Vec<GroupedAffineKey>,
    /// Count rows grouped by the UTC minute of a DateTime column.
    pub grouped_minute: Option<GroupedMinute>,
    /// EXTRACT minute in a three-key grouped count.
    pub grouped_extract_minute: Option<GroupedExtractMinute>,
    /// Conditional string key in a five-key grouped count.
    pub grouped_case_key: Option<GroupedCaseKey>,
    /// REGEXP_REPLACE grouping with length, count and minimum metrics.
    pub grouped_regex: Option<GroupedRegex>,
    /// Computed arguments aligned with grouped aggregate metrics, when present.
    pub grouped_expressions: Vec<Option<ScalarExpr>>,
    /// Whether the grouped metric is used only for HAVING.
    pub hidden_aggregate: bool,
    pub alias: Option<String>,
    /// One wildcard column rename. Applies to query output only.
    pub rename: Option<(String, String)>,
    pub distinct: bool,
    pub source: Option<Source>,
    pub predicates: Vec<Predicate>,
    /// Additional OR branches, each containing AND-connected predicates.
    pub or_predicates: Vec<Vec<Predicate>>,
    pub order_by: Option<OrderBy>,
    pub order_by_extra: Vec<OrderBy>,
    pub having: Option<Having>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl Select {
    /// ORDER BY keys in SQL precedence order.
    pub fn order_terms(&self) -> impl Iterator<Item = &OrderBy> {
        self.order_by.iter().chain(self.order_by_extra.iter())
    }

    /// Map SELECT output aliases to the underlying column used for ORDER BY.
    pub(crate) fn order_source_column<'a>(&'a self, column: &'a str) -> &'a str {
        if let Some((source, target)) = &self.rename {
            if column == target.as_str() {
                return source;
            }
        }
        if self
            .alias
            .as_deref()
            .is_some_and(|alias| alias.eq_ignore_ascii_case(column))
        {
            if let Projection::Columns(columns) = &self.projection {
                if let [source] = columns.as_slice() {
                    return source;
                }
            }
        }
        column
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SQL parse error: {}", self.0)
    }
}

impl Error for ParseError {}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

impl Parser {
    fn take(&mut self, token: Token) -> bool {
        if self.tokens.get(self.index) == Some(&token) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn keyword(&mut self, value: &str) -> bool {
        if matches!(self.tokens.get(self.index), Some(Token::Word(word)) if word.eq_ignore_ascii_case(value))
        {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn like_keyword(&mut self) -> Option<bool> {
        if self.keyword("LIKE") {
            Some(false)
        } else if self.keyword("ILIKE") {
            Some(true)
        } else {
            None
        }
    }

    fn word(&mut self) -> Result<String, ParseError> {
        match self.tokens.get(self.index) {
            Some(Token::Word(value) | Token::QuotedWord(value)) => {
                self.index += 1;
                Ok(value.clone())
            }
            token => Err(ParseError(format!(
                "expected an identifier at token {} (found {token:?})",
                self.index
            ))),
        }
    }

    fn bounded_word(&mut self) -> Result<String, ParseError> {
        let name = self.word()?;
        if name.len() > 128 {
            return Err(ParseError("identifiers cannot exceed 128 bytes".into()));
        }
        Ok(name)
    }

    /// Accept multi-word quoted aliases only at output positions.
    pub(crate) fn output_alias_word(&mut self) -> Result<String, ParseError> {
        if let Some(Token::QuotedAlias(name)) = self.tokens.get(self.index) {
            let name = name.clone();
            self.index += 1;
            return Ok(name);
        }
        self.bounded_word()
    }

    /// Only output positions can consume an alias without AS. Keep clause
    /// keywords out so a missing comma cannot swallow FROM, GROUP BY, etc.
    pub(crate) fn select_output_alias(&mut self) -> Result<Option<String>, ParseError> {
        if self.keyword("AS") {
            return Ok(Some(self.output_alias_word()?));
        }
        let implicit = match self.tokens.get(self.index) {
            Some(Token::QuotedWord(_) | Token::QuotedAlias(_)) => true,
            Some(Token::Word(name)) => ![
                "FROM",
                "PREWHERE",
                "WHERE",
                "GROUP",
                "HAVING",
                "ORDER",
                "LIMIT",
                "OFFSET",
                "FETCH",
                "FORMAT",
                "SETTINGS",
                "RENAME",
                "JOIN",
                "INNER",
                "LEFT",
                "RIGHT",
                "FULL",
                "CROSS",
                "ON",
                "USING",
                "UNION",
                "INTERSECT",
                "EXCEPT",
                "ALL",
                "DISTINCT",
            ]
            .iter()
            .any(|word| name.eq_ignore_ascii_case(word)),
            _ => false,
        };
        if implicit {
            Ok(Some(self.output_alias_word()?))
        } else {
            Ok(None)
        }
    }

    fn literal(&mut self) -> Result<String, ParseError> {
        let mut depth = 0usize;
        while self.take(Token::LParen) {
            depth += 1;
            if depth > 16 {
                return Err(ParseError("literal parentheses exceed 16 levels".into()));
            }
        }
        let literal = self.unparenthesized_literal()?;
        for _ in 0..depth {
            if !self.take(Token::RParen) {
                return Err(ParseError("literal requires closing ')'".into()));
            }
        }
        Ok(literal)
    }

    fn unparenthesized_literal(&mut self) -> Result<String, ParseError> {
        // TPC-H validation SQL commonly expresses date bounds as CAST('...' AS DATE).
        // Resolve only quoted date/timestamp literals; general CAST expressions
        // belong in the typed expression evaluator, not a literal predicate.
        if self.keyword("CAST") {
            if !self.take(Token::LParen) {
                return Err(ParseError("CAST requires '('".into()));
            }
            let value = match self.tokens.get(self.index) {
                Some(Token::String(value)) => value.clone(),
                _ => return Err(ParseError("CAST date requires a quoted literal".into())),
            };
            self.index += 1;
            if !self.keyword("AS") {
                return Err(ParseError("CAST requires AS".into()));
            }
            let target = self.word()?;
            if !target.eq_ignore_ascii_case("DATE") && !target.eq_ignore_ascii_case("TIMESTAMP") {
                return Err(ParseError(
                    "only CAST(... AS DATE/TIMESTAMP) is supported in predicates".into(),
                ));
            }
            if !self.take(Token::RParen) {
                return Err(ParseError("CAST date requires ')'".into()));
            }
            return Ok(value);
        }
        // Typed dates and timestamps use the same storage format as quoted
        // literals. Support the DATE + INTERVAL 'N' YEAR form used by TPC-H Q6.
        let is_date = matches!(
            self.tokens.get(self.index),
            Some(Token::Word(kind)) if kind.eq_ignore_ascii_case("DATE")
        );
        if is_date || self.keyword("TIMESTAMP") {
            if is_date {
                self.index += 1;
            }
            let value = self.tokens.get(self.index);
            let Some(Token::String(value)) = value else {
                return Err(ParseError(
                    "DATE/TIMESTAMP requires a quoted literal".into(),
                ));
            };
            let value = value.clone();
            self.index += 1;
            if is_date {
                let subtract = if self.take(Token::Plus) {
                    Some(false)
                } else if self.take(Token::Minus) {
                    Some(true)
                } else {
                    None
                };
                if let Some(subtract) = subtract {
                    if !self.keyword("INTERVAL") {
                        return Err(ParseError("DATE arithmetic requires INTERVAL".into()));
                    }
                    let amount = match self.tokens.get(self.index) {
                        Some(Token::Number(value) | Token::String(value)) => {
                            value
                                .parse::<i64>()
                                .map_err(|_| ParseError("invalid INTERVAL count".into()))?
                        }
                        _ => return Err(ParseError("INTERVAL requires an integer count".into())),
                    };
                    self.index += 1;
                    let amount = if subtract {
                        amount
                            .checked_neg()
                            .ok_or_else(|| ParseError("INTERVAL count overflow".into()))?
                    } else {
                        amount
                    };
                    if self.keyword("YEAR") {
                        return literal::add_date_years(&value, amount);
                    }
                    if self.keyword("MONTH") {
                        return literal::add_date_months(&value, amount);
                    }
                    if self.keyword("DAY") {
                        return literal::add_date_days(&value, amount);
                    }
                    return Err(ParseError(
                        "only INTERVAL DAY, MONTH or YEAR is supported".into(),
                    ));
                }
            }
            return Ok(value);
        }
        if self.keyword("TRUE") {
            return Ok("1".into());
        }
        if self.keyword("FALSE") {
            return Ok("0".into());
        }
        let is_number = matches!(self.tokens.get(self.index), Some(Token::Number(_)));
        let mut value = match self.tokens.get(self.index) {
            Some(Token::Number(value) | Token::String(value)) => value.clone(),
            _ => return Err(ParseError("expected a SQL literal".into())),
        };
        self.index += 1;
        if is_number {
            loop {
                let subtract = if self.take(Token::Plus) {
                    Some(false)
                } else if self.take(Token::Minus) {
                    Some(true)
                } else {
                    None
                };
                let Some(subtract) = subtract else {
                    break;
                };
                let Some(Token::Number(right)) = self.tokens.get(self.index) else {
                    return Err(ParseError("numeric arithmetic requires a literal".into()));
                };
                value = literal::arithmetic(&value, right, subtract)?;
                self.index += 1;
            }
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<String, ParseError> {
        match self.tokens.get(self.index) {
            Some(Token::Number(value)) => {
                self.index += 1;
                Ok(value.clone())
            }
            _ => Err(ParseError("expected a number".into())),
        }
    }

    fn operator(&mut self) -> Result<Operator, ParseError> {
        match self.tokens.get(self.index) {
            Some(Token::Operator(value)) => {
                self.index += 1;
                Ok(*value)
            }
            _ => Err(ParseError("expected a comparison operator".into())),
        }
    }
    fn order_term(
        &mut self,
        projection: &Projection,
        hidden_aggregate: bool,
    ) -> Result<OrderBy, ParseError> {
        // Positions are 1-based and refer to projected columns.
        let column = if matches!(self.tokens.get(self.index), Some(Token::Number(_))) {
            let position = self
                .number()?
                .parse::<usize>()
                .map_err(|_| ParseError("ORDER BY position is too large".into()))?;
            if hidden_aggregate && position != 1 {
                return Err(ParseError("ORDER BY position is out of range".into()));
            }
            match projection {
                Projection::Columns(columns) => position
                    .checked_sub(1)
                    .and_then(|index| columns.get(index))
                    .cloned()
                    .ok_or_else(|| ParseError("ORDER BY position is out of range".into()))?,
                Projection::GroupedCount { column }
                | Projection::GroupedDistinctCount { column, .. } => match position {
                    1 => column.clone(),
                    2 => "count".into(),
                    _ => return Err(ParseError("ORDER BY position is out of range".into())),
                },
                Projection::GroupedMultiCount { columns }
                | Projection::GroupedMultiDistinctCount { columns, .. } => {
                    if position == columns.len() + 1 {
                        "count".into()
                    } else {
                        position
                            .checked_sub(1)
                            .and_then(|index| columns.get(index))
                            .cloned()
                            .ok_or_else(|| ParseError("ORDER BY position is out of range".into()))?
                    }
                }
                Projection::GroupedMultiAggregates { columns, items, .. } => {
                    if position <= columns.len() {
                        columns[position - 1].clone()
                    } else {
                        let index = position - columns.len() - 1;
                        items.get(index).ok_or_else(|| {
                            ParseError("ORDER BY position is out of range".into())
                        })?;
                        format!("#aggregate{index}")
                    }
                }
                Projection::GroupedAggregate { column, .. } => match position {
                    1 => column.clone(),
                    2 => "#aggregate".into(),
                    _ => {
                        return Err(ParseError(
                            "ORDER BY aggregate output requires SUM, MIN, MAX, AVG or COUNT".into(),
                        ));
                    }
                },
                Projection::GroupedAggregates { column, items, .. } => {
                    if position == 1 {
                        column.clone()
                    } else {
                        let index = position - 2;
                        items.get(index).ok_or_else(|| {
                            ParseError("ORDER BY position is out of range".into())
                        })?;
                        format!("#aggregate{index}")
                    }
                }
                Projection::All => {
                    return Err(ParseError(
                        "ORDER BY positions require explicit projected columns".into(),
                    ));
                }
                _ => return Err(ParseError("ORDER BY requires column projections".into())),
            }
        } else {
            self.output_alias_word()?
        };
        if matches!(
            projection,
            Projection::GroupedCount { .. } | Projection::GroupedMultiCount { .. }
        ) && column.eq_ignore_ascii_case("count")
            && self.take(Token::LParen)
            && !self.take(Token::RParen)
            && !(self.take(Token::Star) && self.take(Token::RParen))
        {
            return Err(ParseError(
                "expected COUNT() or COUNT(*) in ORDER BY".into(),
            ));
        }
        let column = if matches!(
            projection,
            Projection::GroupedCount { .. } | Projection::GroupedMultiCount { .. }
        ) && column.eq_ignore_ascii_case("count")
        {
            "count".to_string()
        } else {
            column
        };
        // Resolve ORDER BY sum(value) to the selected aggregate, not a column.
        let column = if let Projection::GroupedAggregate {
            function,
            value_column,
            ..
        } = projection
        {
            if self.take(Token::LParen) {
                let argument = if self.take(Token::Star)
                    || self.tokens.get(self.index) == Some(&Token::RParen)
                {
                    "*".to_string()
                } else {
                    self.word()?
                };
                if !self.take(Token::RParen) {
                    return Err(ParseError("expected ')' in ORDER BY aggregate".into()));
                }
                if !function.name().eq_ignore_ascii_case(&column) || argument != *value_column {
                    return Err(ParseError("ORDER BY aggregate must match SELECT".into()));
                }
                "#aggregate".into()
            } else {
                column
            }
        } else if let Projection::GroupedAggregates { items, .. }
        | Projection::GroupedMultiAggregates { items, .. } = projection
        {
            if self.take(Token::LParen) {
                let argument = if self.take(Token::Star)
                    || self.tokens.get(self.index) == Some(&Token::RParen)
                {
                    "*".to_string()
                } else {
                    self.word()?
                };
                if !self.take(Token::RParen) {
                    return Err(ParseError("expected ')' in ORDER BY aggregate".into()));
                }
                let index = items
                    .iter()
                    .position(|(function, name)| {
                        function.name().eq_ignore_ascii_case(&column) && name == &argument
                    })
                    .ok_or_else(|| ParseError("ORDER BY aggregate must match SELECT".into()))?;
                format!("#aggregate{index}")
            } else {
                let aliases = match projection {
                    Projection::GroupedAggregates { aliases, .. }
                    | Projection::GroupedMultiAggregates { aliases, .. } => aliases,
                    _ => unreachable!("aggregate aliases belong to grouped projections"),
                };
                let selected_alias = aliases.iter().position(|alias| {
                    alias
                        .as_ref()
                        .is_some_and(|alias| alias.eq_ignore_ascii_case(&column))
                });
                if let Some(index) = selected_alias {
                    format!("#aggregate{index}")
                } else {
                    column
                }
            }
        } else {
            column
        };
        let using_operator = self.keyword("USING");
        let descending = if using_operator {
            match self.operator()? {
                Operator::Lt => false,
                Operator::Gt => true,
                _ => return Err(ParseError("ORDER BY USING requires < or >".into())),
            }
        } else if self.keyword("DESC") {
            true
        } else {
            self.keyword("ASC");
            false
        };
        let nulls_first = if self.keyword("NULLS") {
            if self.keyword("FIRST") {
                Some(true)
            } else if self.keyword("LAST") {
                Some(false)
            } else {
                return Err(ParseError("NULLS requires FIRST or LAST".into()));
            }
        } else if using_operator {
            Some(descending)
        } else {
            None
        };
        Ok(OrderBy {
            column,
            descending,
            nulls_first,
        })
    }
}

// ClickHouse ORDER BY ALL sorts by each selected output column in order.
// The current engine supports at most eight sort keys and cannot expand *.
fn order_by_all_columns(projection: &Projection) -> Result<Vec<String>, ParseError> {
    let mut columns = match projection {
        Projection::Columns(columns) => columns.clone(),
        Projection::GroupedCount { column } | Projection::GroupedDistinctCount { column, .. } => {
            vec![column.clone(), "count".into()]
        }
        Projection::GroupedMultiCount { columns }
        | Projection::GroupedMultiDistinctCount { columns, .. } => {
            let mut result = columns.clone();
            result.push("count".into());
            result
        }
        Projection::GroupedAggregate { column, .. } => vec![column.clone(), "#aggregate".into()],
        Projection::GroupedAggregates { column, items, .. } => {
            let mut result = vec![column.clone()];
            result.extend((0..items.len()).map(|index| format!("#aggregate{index}")));
            result
        }
        Projection::GroupedMultiAggregates { columns, items, .. } => {
            let mut result = columns.clone();
            result.extend((0..items.len()).map(|index| format!("#aggregate{index}")));
            result
        }
        _ => {
            return Err(ParseError(
                "ORDER BY ALL requires explicit projected or grouped columns".into(),
            ));
        }
    };
    if columns.is_empty() || columns.len() > 8 {
        return Err(ParseError(
            "ORDER BY ALL supports at most 8 output columns".into(),
        ));
    }
    Ok(std::mem::take(&mut columns))
}

/// Split a terminal ClickHouse FORMAT clause without confusing SQL string
/// contents or comments with keywords. Return the original SQL when absent.
/// The caller validates the requested format against supported encodings.
pub fn split_clickhouse_format(sql: &str) -> Result<(&str, Option<&str>), ParseError> {
    let trimmed = sql.trim_end();
    let trimmed = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    let tokens = tokenize(trimmed)?;
    let (Some(Token::Word(name)), Some(Token::Word(keyword))) =
        (tokens.last(), tokens.get(tokens.len().saturating_sub(2)))
    else {
        return Ok((sql, None));
    };
    if !keyword.eq_ignore_ascii_case("FORMAT") {
        return Ok((sql, None));
    }
    let Some(format_start) = trimmed.len().checked_sub(name.len()) else {
        return Ok((sql, None));
    };
    let Some(format_name) = trimmed.get(format_start..) else {
        return Ok((sql, None));
    };
    if !format_name.eq_ignore_ascii_case(name) {
        return Ok((sql, None));
    }
    let before_format = trimmed[..format_start].trim_end();
    let Some(keyword_start) = before_format.len().checked_sub(6) else {
        return Ok((sql, None));
    };
    if !before_format[keyword_start..].eq_ignore_ascii_case("FORMAT") {
        return Ok((sql, None));
    }
    let statement = before_format[..keyword_start].trim_end();
    if statement.is_empty() {
        return Err(ParseError("FORMAT requires a SELECT statement".into()));
    }
    Ok((statement, Some(format_name)))
}

pub fn parse_select(sql: &str) -> Result<Select, ParseError> {
    let mut tokens = tokenize(sql)?;
    if matches!(tokens.as_slice(), [Token::Word(select), Token::Word(function), Token::LParen, ..] if select.eq_ignore_ascii_case("SELECT") && function.eq_ignore_ascii_case("REGEXP_REPLACE"))
    {
        return match grouped_regex::parse(tokens.clone())? {
            Some(grouped) => Ok(grouped),
            None => parse_select_tokens(tokens),
        };
    }

    if matches!(tokens.as_slice(), [Token::Word(select), Token::Word(_), Token::Comma, Token::Word(_), Token::Comma, Token::Word(_), Token::Comma, Token::Word(case), ..] if select.eq_ignore_ascii_case("SELECT") && case.eq_ignore_ascii_case("CASE"))
    {
        return match grouped_case::parse(tokens.clone())? {
            Some(grouped) => Ok(grouped),
            None => parse_select_tokens(tokens),
        };
    }

    if matches!(tokens.as_slice(), [Token::Word(select), Token::Word(_), Token::Comma, Token::Word(function), Token::LParen, ..] if select.eq_ignore_ascii_case("SELECT") && function.eq_ignore_ascii_case("EXTRACT"))
    {
        return match grouped_extract::parse(tokens.clone())? {
            Some(grouped) => Ok(grouped),
            None => parse_select_tokens(tokens),
        };
    }

    if matches!(tokens.as_slice(), [Token::Word(select), Token::Word(function), Token::LParen, ..] if select.eq_ignore_ascii_case("SELECT") && function.eq_ignore_ascii_case("DATE_TRUNC"))
    {
        return match grouped_minute::parse(tokens.clone())? {
            Some(grouped) => Ok(grouped),
            None => parse_select_tokens(tokens),
        };
    }
    if matches!(tokens.as_slice(), [Token::Word(select), Token::Word(key), Token::Comma, Token::Word(again), Token::Minus | Token::Plus, Token::Number(_), ..] if select.eq_ignore_ascii_case("SELECT") && key == again)
    {
        return match grouped_affine::parse(tokens.clone())? {
            Some(grouped) => Ok(grouped),
            None => parse_select_tokens(tokens),
        };
    }
    let literal = match tokens.as_slice() {
        [
            Token::Word(select),
            Token::Number(number),
            Token::Comma,
            Token::Word(_),
            ..,
        ] if select.eq_ignore_ascii_case("SELECT") => number.parse::<i64>().ok(),
        _ => None,
    };
    if let Some(literal) = literal {
        let mut depth = 0usize;
        let mut group = None;
        for (index, token) in tokens.iter().enumerate() {
            match token {
                Token::LParen => depth += 1,
                Token::RParen => depth = depth.saturating_sub(1),
                Token::Word(word)
                    if depth == 0
                        && word.eq_ignore_ascii_case("GROUP")
                        && matches!(tokens.get(index + 1), Some(Token::Word(by)) if by.eq_ignore_ascii_case("BY")) =>
                {
                    group = Some(index);
                    break;
                }
                _ => {}
            }
        }
        if let Some(group) = group {
            if matches!(tokens.get(group + 2), Some(Token::Number(n)) if n == "1")
                && matches!(tokens.get(group + 3), Some(Token::Comma))
            {
                let mut index = group + 4;
                loop {
                    if let Some(Token::Number(n)) = tokens.get_mut(index) {
                        let pos = n
                            .parse::<usize>()
                            .map_err(|_| ParseError("GROUP BY position is too large".into()))?;
                        if pos <= 1 {
                            return Err(ParseError(
                                "duplicate or invalid GROUP BY position".into(),
                            ));
                        }
                        *n = (pos - 1).to_string();
                    }
                    index += 1;
                    if !matches!(tokens.get(index), Some(Token::Comma)) {
                        break;
                    }
                    index += 1;
                }
                let mut depth = 0usize;
                let mut order_start = None;
                for (index, token) in tokens.iter().enumerate() {
                    match token {
                        Token::LParen => depth += 1,
                        Token::RParen => depth = depth.saturating_sub(1),
                        Token::Word(word)
                            if depth == 0
                                && word.eq_ignore_ascii_case("ORDER")
                                && matches!(tokens.get(index + 1), Some(Token::Word(by)) if by.eq_ignore_ascii_case("BY")) =>
                        {
                            order_start = Some(index + 2);
                            break;
                        }
                        _ => {}
                    }
                }
                if let Some(mut index) = order_start {
                    let mut depth = 0usize;
                    let mut term_start = true;
                    while index < tokens.len() {
                        match &mut tokens[index] {
                            Token::Word(word)
                                if depth == 0
                                    && ["LIMIT", "OFFSET", "FETCH", "FORMAT"]
                                        .iter()
                                        .any(|end| word.eq_ignore_ascii_case(end)) =>
                            {
                                break;
                            }
                            Token::Semicolon if depth == 0 => break,
                            Token::Number(number) if depth == 0 && term_start => {
                                let position = number.parse::<usize>().map_err(|_| {
                                    ParseError("ORDER BY position is too large".into())
                                })?;
                                if position <= 1 {
                                    return Err(ParseError(
                                        "ORDER BY constant position is unsupported".into(),
                                    ));
                                }
                                *number = (position - 1).to_string();
                                term_start = false;
                            }
                            Token::Comma if depth == 0 => term_start = true,
                            Token::LParen => {
                                depth += 1;
                                term_start = false;
                            }
                            Token::RParen => depth = depth.saturating_sub(1),
                            _ => term_start = false,
                        }
                        index += 1;
                    }
                }
                tokens.drain(group + 2..group + 4);
                tokens.drain(1..3);
                let mut select = parse_select_tokens(tokens)?;
                if !matches!(
                    select.projection,
                    Projection::GroupedCount { .. }
                        | Projection::GroupedMultiCount { .. }
                        | Projection::GroupedDistinctCount { .. }
                        | Projection::GroupedMultiDistinctCount { .. }
                        | Projection::GroupedAggregate { .. }
                        | Projection::GroupedAggregates { .. }
                        | Projection::GroupedMultiAggregates { .. }
                ) {
                    return Err(ParseError(
                        "constant grouped projection requires grouped aggregates".into(),
                    ));
                }
                select.grouped_literal = Some(literal);
                return Ok(select);
            }
        }
    }
    parse_select_tokens(tokens)
}

// COUNT(*), COUNT(), and COUNT(non-NULL constants) share a grouped marker.
// The executor counts rows without reading a synthetic data column.
// A lone distinct count uses the specialized executor. A following metric
// must instead use the multi-aggregate plan.
fn distinct_count_has_following_metric(parser: &Parser) -> bool {
    let mut index = parser.index + 5;
    if matches!(parser.tokens.get(index), Some(Token::Word(word)) if word.eq_ignore_ascii_case("AS"))
    {
        index += 2;
    }
    parser.tokens.get(index) == Some(&Token::Comma)
}

fn grouped_metric_filter(parser: &mut Parser) -> Result<GroupedFilter, ParseError> {
    let column = parser.bounded_word()?;
    if let Some(Token::Operator(operator)) = parser.tokens.get(parser.index) {
        let operator = *operator;
        if !matches!(
            operator,
            Operator::Eq | Operator::Ne | Operator::Lt | Operator::Le | Operator::Gt | Operator::Ge
        ) {
            return Err(ParseError(
                "unsupported conditional aggregate comparison".into(),
            ));
        }
        parser.index += 1;
        let literal = parser.literal()?;
        Ok(GroupedFilter::Comparison {
            column,
            operator,
            literal,
        })
    } else {
        Ok(GroupedFilter::Truthy(column))
    }
}

fn grouped_metric_column(
    parser: &mut Parser,
    function: AggregateFunction,
) -> Result<String, ParseError> {
    if function == AggregateFunction::Count {
        if parser.keyword("DISTINCT") {
            return Err(ParseError(
                "mixed grouped COUNT(DISTINCT) is not yet supported".into(),
            ));
        }
        if parser.take(Token::Star) || parser.tokens.get(parser.index) == Some(&Token::RParen) {
            return Ok("*".into());
        }
        if matches!(
            parser.tokens.get(parser.index),
            Some(Token::Number(_) | Token::String(_))
        ) {
            // COUNT(non-NULL constant) is equivalent to counting each row.
            parser.index += 1;
            return Ok("*".into());
        }
    }
    let column = parser.word()?;
    if matches!(
        parser.tokens.get(parser.index),
        Some(Token::Star | Token::Slash | Token::Percent | Token::Plus | Token::Minus)
    ) {
        return Err(ParseError(
            "computed grouped aggregate arguments are not yet supported".into(),
        ));
    }
    Ok(column)
}

fn grouped_metric_with_expression(
    parser: &mut Parser,
    function: AggregateFunction,
) -> Result<(String, Option<GroupedFilter>, Option<ScalarExpr>), ParseError> {
    let start = parser.index;
    if function != AggregateFunction::Count
        && !matches!(parser.tokens.get(start), Some(Token::Word(word)) if word.eq_ignore_ascii_case("CASE"))
    {
        let expression = expression::parse_expression_tokens(parser)?;
        if !matches!(expression, ScalarExpr::Column(_)) {
            return Ok((
                multi_aggregate::render_expr(&expression),
                None,
                Some(expression),
            ));
        }
        parser.index = start;
    }
    let (metric, filter) = grouped_metric_value(parser, function)?;
    Ok(match metric {
        GroupedMetricValue::Count => ("*".into(), filter, None),
        GroupedMetricValue::Column(column) => (column, filter, None),
        GroupedMetricValue::Expression(expression) => (
            multi_aggregate::render_expr(&expression),
            filter,
            Some(expression),
        ),
    })
}

fn grouped_join_column(parser: &mut Parser) -> Result<String, ParseError> {
    let first = parser.bounded_word()?;
    if parser.take(Token::Dot) {
        Ok(format!("{first}\0{}", parser.bounded_word()?))
    } else {
        Ok(first)
    }
}

// SUM(CASE WHEN ... THEN metric ELSE 0 END) is equivalent to a
// conditional SUM of that metric. Reject other CASE shapes rather than
// silently dropping a nonzero ELSE branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GroupedMetricValue {
    Count,
    Column(String),
    Expression(ScalarExpr),
}

fn numeric_grouped_case_value(expression: &ScalarExpr) -> bool {
    match expression {
        ScalarExpr::Column(_)
        | ScalarExpr::TextLength(_)
        | ScalarExpr::Integer(_)
        | ScalarExpr::Decimal(_) => true,
        ScalarExpr::TextLiteral(_)
        | ScalarExpr::TextConcat(_)
        | ScalarExpr::TextSubstring { .. } => false,
        ScalarExpr::Round { value, .. } => numeric_grouped_case_value(value),
        ScalarExpr::Add(left, right)
        | ScalarExpr::Subtract(left, right)
        | ScalarExpr::Multiply(left, right)
        | ScalarExpr::Divide(left, right)
        | ScalarExpr::Modulo(left, right) => {
            numeric_grouped_case_value(left) && numeric_grouped_case_value(right)
        }
        ScalarExpr::Case {
            branches,
            else_expr,
        } => {
            branches
                .iter()
                .all(|(_, value)| numeric_grouped_case_value(value))
                && else_expr
                    .as_ref()
                    .is_none_or(|value| numeric_grouped_case_value(value))
        }
        ScalarExpr::Compare(..)
        | ScalarExpr::IsNull { .. }
        | ScalarExpr::TextCompare { .. }
        | ScalarExpr::TextLike { .. }
        | ScalarExpr::And(..)
        | ScalarExpr::Or(..) => false,
    }
}

fn grouped_case_value(parser: &mut Parser) -> Result<GroupedMetricValue, ParseError> {
    if matches!(parser.tokens.get(parser.index), Some(Token::Number(value)) if value == "1") {
        parser.index += 1;
        return Ok(GroupedMetricValue::Count);
    }
    let start = parser.index;
    let mut depth = 0usize;
    let mut end = None;
    for index in start..parser.tokens.len() {
        match &parser.tokens[index] {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(word)
                if depth == 0
                    && (word.eq_ignore_ascii_case("ELSE") || word.eq_ignore_ascii_case("END")) =>
            {
                end = Some(index);
                break;
            }
            _ => {}
        }
    }
    let end = end.ok_or_else(|| ParseError("grouped CASE requires ELSE or END".into()))?;
    if end == start {
        return Err(ParseError("grouped CASE requires a numeric value".into()));
    }
    let (tokens, _) = join::encode_qualified_expression_tokens(&parser.tokens[start..end]);
    let mut expression_parser = Parser { tokens, index: 0 };
    let expression = expression::parse_expression_tokens(&mut expression_parser)?;
    if expression_parser.index != expression_parser.tokens.len() {
        return Err(ParseError(
            "unsupported grouped CASE value expression".into(),
        ));
    }
    if !numeric_grouped_case_value(&expression) {
        return Err(ParseError("grouped CASE requires a numeric value".into()));
    }
    parser.index = end;
    match expression {
        ScalarExpr::Column(column) => Ok(GroupedMetricValue::Column(column)),
        expression => Ok(GroupedMetricValue::Expression(expression)),
    }
}

fn grouped_metric_value(
    parser: &mut Parser,
    function: AggregateFunction,
) -> Result<(GroupedMetricValue, Option<GroupedFilter>), ParseError> {
    if !parser.keyword("CASE") {
        let column = grouped_metric_column(parser, function)?;
        return Ok((
            if column == "*" {
                GroupedMetricValue::Count
            } else {
                GroupedMetricValue::Column(column)
            },
            None,
        ));
    }
    if function != AggregateFunction::Sum || !parser.keyword("WHEN") {
        return Err(ParseError(
            "grouped CASE requires SUM(CASE WHEN ...)".into(),
        ));
    }
    let mut depth = 0usize;
    let mut has_boolean_chain = false;
    let mut condition_end = None;
    for index in parser.index..parser.tokens.len() {
        match &parser.tokens[index] {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(word) if depth == 0 && word.eq_ignore_ascii_case("THEN") => {
                condition_end = Some(index);
                break;
            }
            Token::Word(word)
                if depth == 0
                    && (word.eq_ignore_ascii_case("AND") || word.eq_ignore_ascii_case("OR")) =>
            {
                has_boolean_chain = true;
            }
            _ => {}
        }
    }
    if has_boolean_chain {
        let end = condition_end.ok_or_else(|| ParseError("grouped CASE requires THEN".into()))?;
        let source = &parser.tokens[parser.index..end];
        let (tokens, _) = join::encode_qualified_expression_tokens(source);
        let mut condition_parser = Parser { tokens, index: 0 };
        let condition = expression::parse_expression_tokens(&mut condition_parser)?;
        if condition_parser.index != condition_parser.tokens.len() {
            return Err(ParseError("unsupported grouped CASE condition".into()));
        }
        if !matches!(
            condition,
            ScalarExpr::Compare(..)
                | ScalarExpr::TextCompare { .. }
                | ScalarExpr::TextLike { .. }
                | ScalarExpr::IsNull { .. }
                | ScalarExpr::And(..)
                | ScalarExpr::Or(..)
        ) {
            return Err(ParseError(
                "grouped CASE requires a comparison condition".into(),
            ));
        }
        parser.index = end + 1;
        let metric = grouped_case_value(parser)?;
        let unit_count = matches!(metric, GroupedMetricValue::Count);
        let mut else_zero = false;
        if parser.keyword("ELSE") && !parser.keyword("NULL") {
            let else_value = parser.literal()?;
            if else_value != "0" && else_value != "0.0" {
                return Err(ParseError(
                    "grouped SUM(CASE ...) requires ELSE 0 or NULL".into(),
                ));
            }
            else_zero = true;
        }
        if !parser.keyword("END") {
            return Err(ParseError("grouped CASE requires END".into()));
        }
        if unit_count && !else_zero {
            return Err(ParseError("SUM(CASE ... THEN 1) requires ELSE 0".into()));
        }
        return Ok((
            metric,
            Some(GroupedFilter::CaseExpression {
                condition,
                else_zero,
            }),
        ));
    }
    if matches!(parser.tokens.get(parser.index), Some(Token::LParen)) {
        let condition = expression::parse_expression_tokens(parser)?;
        if !matches!(
            condition,
            ScalarExpr::Compare(..)
                | ScalarExpr::TextCompare { .. }
                | ScalarExpr::TextLike { .. }
                | ScalarExpr::IsNull { .. }
                | ScalarExpr::And(..)
                | ScalarExpr::Or(..)
        ) {
            return Err(ParseError(
                "grouped CASE requires a comparison condition".into(),
            ));
        }
        if !parser.keyword("THEN") {
            return Err(ParseError("grouped CASE requires THEN".into()));
        }
        let metric = grouped_case_value(parser)?;
        let unit_count = matches!(metric, GroupedMetricValue::Count);
        let mut else_zero = false;
        if parser.keyword("ELSE") && !parser.keyword("NULL") {
            let else_value = parser.literal()?;
            if else_value != "0" && else_value != "0.0" {
                return Err(ParseError(
                    "grouped SUM(CASE ...) requires ELSE 0 or NULL".into(),
                ));
            }
            else_zero = true;
        }
        if !parser.keyword("END") {
            return Err(ParseError("grouped CASE requires END".into()));
        }
        if unit_count && !else_zero {
            return Err(ParseError("SUM(CASE ... THEN 1) requires ELSE 0".into()));
        }
        return Ok((
            metric,
            Some(GroupedFilter::CaseExpression {
                condition,
                else_zero,
            }),
        ));
    }
    let column = grouped_join_column(parser)?;
    if parser.keyword("IS") {
        let negated = parser.keyword("NOT");
        if !parser.keyword("NULL") {
            return Err(ParseError("grouped CASE IS requires NULL".into()));
        }
        if !parser.keyword("THEN") {
            return Err(ParseError("grouped CASE requires THEN".into()));
        }
        let metric = grouped_case_value(parser)?;
        let unit_count = matches!(metric, GroupedMetricValue::Count);
        let mut else_zero = false;
        if parser.keyword("ELSE") && !parser.keyword("NULL") {
            let else_value = parser.literal()?;
            if else_value != "0" && else_value != "0.0" {
                return Err(ParseError(
                    "grouped SUM(CASE ...) requires ELSE 0 or NULL".into(),
                ));
            }
            else_zero = true;
        }
        if !parser.keyword("END") {
            return Err(ParseError("grouped CASE requires END".into()));
        }
        if unit_count && !else_zero {
            return Err(ParseError("SUM(CASE ... THEN 1) requires ELSE 0".into()));
        }
        return Ok((metric, Some(GroupedFilter::CaseNull { column, negated })));
    }
    let operator = parser.operator()?;
    if !matches!(
        operator,
        Operator::Eq | Operator::Ne | Operator::Lt | Operator::Le | Operator::Gt | Operator::Ge
    ) {
        return Err(ParseError(
            "grouped CASE requires a comparison condition".into(),
        ));
    }
    let literal = parser.literal()?;
    let mut comparisons = vec![(operator, literal.clone())];
    let mut all = None;
    loop {
        let conjunction = if parser.keyword("AND") {
            true
        } else if parser.keyword("OR") {
            false
        } else {
            break;
        };
        if all.is_some_and(|previous| previous != conjunction) {
            return Err(ParseError(
                "grouped CASE cannot mix AND and OR without parentheses".into(),
            ));
        }
        all = Some(conjunction);
        if comparisons.len() >= 8 {
            return Err(ParseError(
                "grouped CASE supports at most 8 comparisons".into(),
            ));
        }
        let next_column = grouped_join_column(parser)?;
        if next_column != column {
            return Err(ParseError(
                "grouped CASE comparisons must use the same column".into(),
            ));
        }
        let next_operator = parser.operator()?;
        if !matches!(
            next_operator,
            Operator::Eq | Operator::Ne | Operator::Lt | Operator::Le | Operator::Gt | Operator::Ge
        ) {
            return Err(ParseError(
                "grouped CASE requires a comparison condition".into(),
            ));
        }
        comparisons.push((next_operator, parser.literal()?));
    }
    if !parser.keyword("THEN") {
        return Err(ParseError("grouped CASE requires THEN".into()));
    }
    // SUM(CASE ... THEN 1 ELSE 0 END) counts matching rows, including
    // groups with zero matches. Reuse the filtered COUNT(*) state.
    let metric = grouped_case_value(parser)?;
    let unit_count = matches!(metric, GroupedMetricValue::Count);
    let mut else_zero = false;
    if parser.keyword("ELSE") && !parser.keyword("NULL") {
        let else_value = parser.literal()?;
        if else_value != "0" && else_value != "0.0" {
            return Err(ParseError(
                "grouped SUM(CASE ...) requires ELSE 0 or NULL".into(),
            ));
        }
        else_zero = true;
    }
    if !parser.keyword("END") {
        return Err(ParseError("grouped CASE requires END".into()));
    }
    if unit_count && !else_zero {
        return Err(ParseError("SUM(CASE ... THEN 1) requires ELSE 0".into()));
    }
    let filter = if let Some(all) = all {
        if !unit_count || !else_zero {
            return Err(ParseError(
                "combined grouped CASE requires THEN 1 ELSE 0".into(),
            ));
        }
        GroupedFilter::CaseElseZeroCombined {
            column,
            comparisons,
            all,
        }
    } else if else_zero {
        GroupedFilter::CaseElseZero {
            column,
            operator,
            literal,
        }
    } else {
        GroupedFilter::Comparison {
            column,
            operator,
            literal,
        }
    };
    Ok((metric, Some(filter)))
}


#[cfg(test)]
mod tests;

#[cfg(test)]
mod group_by_position_tests {
    use super::*;

    #[test]
    fn resolves_first_projected_group_key() {
        for sql in [
            "SELECT number, COUNT(*) FROM numbers(4) GROUP BY 1",
            "SELECT number, SUM(number) FROM numbers(4) GROUP BY 1",
        ] {
            assert!(parse_select(sql).is_ok(), "{sql}");
        }
    }

    #[test]
    fn rejects_out_of_range_positions() {
        for position in ["0", "2", "999"] {
            let sql = format!("SELECT number, COUNT(*) FROM numbers(4) GROUP BY {position}");
            assert!(parse_select(&sql).is_err(), "{sql}");
        }
    }
}

#[cfg(test)]
mod generated_numbers_boundary_regressions {
    use super::*;

    #[test]
    fn accepts_ranges_ending_at_u64_max() {
        for (sql, expected) in [
            (
                "SELECT number FROM numbers(18446744073709551615, 1)",
                "number\n18446744073709551615\n",
            ),
            (
                "SELECT number FROM numbers(18446744073709551610, 10, 5)",
                "number\n18446744073709551610\n18446744073709551615\n",
            ),
            (
                "SELECT number FROM numbers(18446744073709551615, 0)",
                "number\n",
            ),
        ] {
            let query = parse_select(sql).unwrap();
            let mut output = Vec::new();
            execute(&query, None, &mut output).unwrap();
            assert_eq!(String::from_utf8(output).unwrap(), expected);
        }
    }

    #[test]
    fn rejects_ranges_whose_last_value_overflows() {
        for sql in [
            "SELECT number FROM numbers(18446744073709551615, 2)",
            "SELECT number FROM numbers(18446744073709551614, 5, 2)",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }
}

#[cfg(test)]
mod stddev_clickhouse_parity_tests {
    use super::*;
    #[test]
    fn stddev_alias_matches_sample_standard_deviation() {
        for input in [
            "SELECT stddev(number) FROM numbers(3)",
            "SELECT number, STDDEV(number) FROM numbers(3) GROUP BY number",
        ] {
            let canonical = input
                .replace("stddev(", "stddevSamp(")
                .replace("STDDEV(", "stddevSamp(");
            assert_eq!(
                parse_select(input).unwrap().projection,
                parse_select(&canonical).unwrap().projection
            );
        }
        assert!(parse_select("SELECT stddev() FROM numbers(3)").is_err());
    }
}

#[cfg(test)]
mod tpch_cast_date_tests {
    use super::{Operator, Predicate, parse_select};

    #[test]
    fn accepts_cast_date_predicates_used_by_full_tpch_queries() {
        for sql in [
            "SELECT count(*) FROM lineitem WHERE l_shipdate <= CAST('1998-09-02' AS DATE)",
            "SELECT count(*) FROM lineitem WHERE l_shipdate >= cast('1994-01-01' as date)",
        ] {
            let selected = parse_select(sql).unwrap();
            assert!(matches!(
                &selected.predicates[0],
                Predicate::Comparison {
                    operator: Operator::Le | Operator::Ge,
                    value,
                    ..
                } if value.starts_with("199")
            ));
        }
    }

    #[test]
    fn rejects_incomplete_or_untyped_casts_in_predicates() {
        for value in [
            "CAST('1998-09-02' AS Int64)",
            "CAST('1998-09-02' DATE)",
            "CAST(123 AS DATE)",
            "CAST('1998-09-02' AS DATE",
        ] {
            let sql = format!("SELECT count(*) FROM lineitem WHERE l_shipdate < {value}");
            assert!(parse_select(&sql).is_err(), "{sql}");
        }
    }
}

#[cfg(test)]
mod tpch_interval_unit_tests {
    use super::{Operator, Predicate, parse_select};

    #[test]
    fn folds_standard_tpch_date_intervals_in_where() {
        for (sql, expected) in [
            (
                "SELECT count(*) FROM lineitem WHERE l_shipdate <= DATE '1998-12-01' - INTERVAL '90' DAY",
                "1998-09-02",
            ),
            (
                "SELECT count(*) FROM orders WHERE o_orderdate < DATE '1994-01-01' + INTERVAL '3' MONTH",
                "1994-04-01",
            ),
            (
                "SELECT count(*) FROM lineitem WHERE l_shipdate < DATE '1994-01-01' + INTERVAL '1' YEAR",
                "1995-01-01",
            ),
        ] {
            let selected = parse_select(sql).unwrap();
            assert!(matches!(
                &selected.predicates[0],
                Predicate::Comparison { operator: Operator::Le | Operator::Lt, value, .. }
                    if value == expected
            ));
        }
    }
}

#[cfg(test)]
mod where_boolean_literal_tests {
    use super::*;

    #[test]
    fn where_compares_boolean_literals_as_numeric_values() {
        for (keyword, numeric) in [("TRUE", "1"), ("FALSE", "0"), ("tRuE", "1")] {
            let boolean_sql = format!("SELECT number FROM numbers(3) WHERE number = {keyword}");
            let numeric_sql = format!("SELECT number FROM numbers(3) WHERE number = {numeric}");
            assert_eq!(
                parse_select(&boolean_sql).unwrap(),
                parse_select(&numeric_sql).unwrap()
            );
        }
    }
}

#[cfg(test)]
mod parenthesized_predicate_literal_tests {
    use super::parse_select;

    #[test]
    fn parentheses_do_not_change_literal_comparisons() {
        for (wrapped, bare) in [
            ("number = ((2))", "number = 2"),
            ("number BETWEEN (1) AND (3)", "number BETWEEN 1 AND 3"),
            ("number IN ((1), (2))", "number IN (1, 2)"),
            ("name = ('Ada')", "name = 'Ada'"),
            ("d = (DATE '2000-01-01')", "d = DATE '2000-01-01'"),
        ] {
            let sql = |condition: &str| format!("SELECT count(*) FROM t WHERE {condition}");
            assert_eq!(
                parse_select(&sql(wrapped)).unwrap(),
                parse_select(&sql(bare)).unwrap(),
                "{wrapped}"
            );
        }
    }

    #[test]
    fn malformed_or_deeply_parenthesized_literals_fail() {
        assert!(parse_select("SELECT count(*) FROM t WHERE number = (2 + 1)").is_ok());
        for expression in ["(2", "(())"] {
            let sql = format!("SELECT count(*) FROM t WHERE number = {expression}");
            assert!(parse_select(&sql).is_err(), "{sql}");
        }
        let nested = format!("{}1{}", "(".repeat(17), ")".repeat(17));
        assert!(parse_select(&format!("SELECT count(*) FROM t WHERE number = {nested}")).is_err());
    }
}

#[cfg(test)]
mod tpch_parser_first_stop_7 {
    use super::parse_select;

    #[test]
    fn reports_the_unsupported_sql_feature() {
        let err = parse_select("SELECT 100.00 * sum(l_extendedprice) FROM lineitem").unwrap_err();
        assert!(
            err.0.contains("arithmetic after SELECT decimal constants"),
            "{err}"
        );
    }
}

#[cfg(test)]
mod tpch_parser_first_stop_6 {
    use super::parse_select;

    #[test]
    fn reports_the_unsupported_sql_feature() {
        let err = parse_select("SELECT sum(l_extendedprice) / 7.0 AS avg_yearly FROM lineitem")
            .unwrap_err();
        assert!(err.0.contains("arithmetic on aggregate results"), "{err}");
    }
}

#[cfg(test)]
mod tpch_parser_first_stop_5 {
    use super::parse_select;

    #[test]
    fn reports_the_unsupported_sql_feature() {
        let err = parse_select("SELECT nation FROM (SELECT n_name AS nation FROM nation) AS x")
            .unwrap_err();
        assert!(err.0.contains("derived tables in FROM"), "{err}");
    }
}

#[cfg(test)]
mod tpch_parser_first_stop_4 {
    use super::parse_select;

    #[test]
    fn unsupported_construct_has_specific_error() {
        let error = parse_select(
            "WITH revenue AS (SELECT supplier_no FROM supplier) SELECT supplier_no FROM revenue",
        )
        .unwrap_err();
        assert!(error.0.contains("WITH common table expressions"), "{error}");
    }
}

#[cfg(test)]
mod mixed_grouped_distinct_parser {
    use super::parse_select;

    #[test]
    fn accepts_mixed_distinct_metrics() {
        parse_select("SELECT RegionID, SUM(AdvEngineID), COUNT(*) AS c, AVG(ResolutionWidth), COUNT(DISTINCT UserID) FROM hits GROUP BY RegionID ORDER BY c DESC LIMIT 10").unwrap();
    }
}

#[cfg(test)]
mod tpch_parser_first_stop_2 {
    use super::parse_select;

    #[test]
    fn unsupported_construct_has_specific_error() {
        parse_select("SELECT category, SUM(price * (1 - discount)) FROM sales GROUP BY category")
            .unwrap();
        parse_select(
            "SELECT category, SUM(price * (1 - discount)) AS total FROM sales GROUP BY category",
        )
        .unwrap();
    }
}

#[cfg(test)]
mod tpch_comma_from_diagnostics {
    use super::parse_select;

    #[test]
    fn comma_sources_are_not_reported_as_missing_group_by() {
        let sql = "SELECT l_shipmode, sum(CASE WHEN priority = 'hi' THEN 1 ELSE 0 END) FROM orders, lineitem GROUP BY l_shipmode";
        let error = parse_select(sql).unwrap_err();
        assert!(error.0.contains("comma-separated FROM tables"), "{error}");
    }
}
