mod copy;
mod ddl;
mod delete;
mod date_scalar;
mod execute;
mod explain;
mod expression;
mod insert;
mod insert_select;
mod join;
mod json;
pub use json::parse_json_each_row_fields;
mod lexer;
mod literal;
mod multi_aggregate;
mod scalar;
mod text_scalar;
mod update;
mod where_parser;
pub use copy::{CopyFormat, CopyFrom, parse_copy_from};
pub use ddl::{ColumnDef, Ddl, parse_ddl};
pub use delete::parse_delete;
pub use execute::{ExecutionError, RowPredicate, execute, execute_join, execute_multi_join, merge_uniq_states};
pub use explain::explain_select;
pub use expression::{ScalarExpr, parse_scalar_expression};
pub use insert::{Insert, parse_insert};
pub use insert_select::{InsertSelect, parse_insert_select};
pub use join::{JoinAggregate, JoinColumn, JoinSelect, JoinStep, JoinType, explain_join, parse_join};
pub use multi_aggregate::AggregateItem;
pub use update::{Update, parse_update};

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
        filters: Vec<Option<String>>,
    },
    GroupedMultiAggregates {
        columns: Vec<String>,
        items: Vec<(AggregateFunction, String)>,
        aliases: Vec<Option<String>>,
        filters: Vec<Option<String>>,
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
}

impl Predicate {
    fn column(&self) -> &str {
        match self {
            Self::Comparison { column, .. }
            | Self::In { column, .. }
            | Self::NotIn { column, .. }
            | Self::NotBetween { column, .. }
            | Self::IsNull { column, .. }
            | Self::DistinctFrom { column, .. }
            | Self::Like { column, .. } => column,
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
        if self.alias.as_deref() == Some(column) {
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
            _ => Err(ParseError("expected an identifier".into())),
        }
    }

    fn bounded_word(&mut self) -> Result<String, ParseError> {
        let name = self.word()?;
        if name.len() > 128 {
            return Err(ParseError("identifiers cannot exceed 128 bytes".into()));
        }
        Ok(name)
    }

    fn literal(&mut self) -> Result<String, ParseError> {
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
            if !target.eq_ignore_ascii_case("DATE")
                && !target.eq_ignore_ascii_case("TIMESTAMP")
            {
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
                        Some(Token::Number(value) | Token::String(value)) => value
                            .parse::<i64>()
                            .map_err(|_| ParseError("invalid INTERVAL count".into()))?,
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
        let is_number = matches!(self.tokens.get(self.index), Some(Token::Number(_)));
        let value = match self.tokens.get(self.index) {
            Some(Token::Number(value) | Token::String(value)) => value.clone(),
            _ => return Err(ParseError("expected a SQL literal".into())),
        };
        self.index += 1;
        if is_number {
            let subtract = if self.take(Token::Plus) {
                Some(false)
            } else if self.take(Token::Minus) {
                Some(true)
            } else {
                None
            };
            if let Some(subtract) = subtract {
                let Some(Token::Number(right)) = self.tokens.get(self.index) else {
                    return Err(ParseError("numeric arithmetic requires a literal".into()));
                };
                let result = literal::arithmetic(&value, right, subtract)?;
                self.index += 1;
                return Ok(result);
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
    fn order_term(&mut self, projection: &Projection) -> Result<OrderBy, ParseError> {
        // Positions are 1-based and refer to projected columns.
        let column = if matches!(self.tokens.get(self.index), Some(Token::Number(_))) {
            let position = self
                .number()?
                .parse::<usize>()
                .map_err(|_| ParseError("ORDER BY position is too large".into()))?;
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
                },
                Projection::GroupedAggregate {
                    column, ..
                } => match position {
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
                },
                Projection::All => {
                    return Err(ParseError(
                        "ORDER BY positions require explicit projected columns".into(),
                    ));
                }
                _ => return Err(ParseError("ORDER BY requires column projections".into())),
            }
        } else {
            self.word()?
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
        let column = if let Projection::GroupedAggregates { items, .. }
            | Projection::GroupedMultiAggregates { items, .. } = projection
        {
            if self.take(Token::LParen) {
                let argument = self.word()?;
                if !self.take(Token::RParen) {
                    return Err(ParseError("expected ')' in ORDER BY aggregate".into()));
                }
                let index = items.iter().position(|(function, name)| {
                    function.name().eq_ignore_ascii_case(&column)
                        && name == &argument
                        
                }).ok_or_else(|| ParseError("ORDER BY aggregate must match SELECT".into()))?;
                format!("#aggregate{index}")
            } else {
                let aliases = match projection {
                    Projection::GroupedAggregates { aliases, .. }
                    | Projection::GroupedMultiAggregates { aliases, .. } => aliases,
                    _ => unreachable!("aggregate aliases belong to grouped projections"),
                };
                let selected_alias = aliases.iter().position(|alias| alias.as_ref()
                    .is_some_and(|alias| alias.eq_ignore_ascii_case(&column)));
                if let Some(index) = selected_alias {

                    format!("#aggregate{index}")
                } else {
                    column
                }
            }
        } else {
            column
        };
        let descending = if self.keyword("DESC") {
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
        Projection::GroupedCount { column }
        | Projection::GroupedDistinctCount { column, .. } => vec![column.clone(), "count".into()],
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
        _ => return Err(ParseError(
            "ORDER BY ALL requires explicit projected or grouped columns".into(),
        )),
    };
    if columns.is_empty() || columns.len() > 8 {
        return Err(ParseError("ORDER BY ALL supports at most 8 output columns".into()));
    }
    Ok(std::mem::take(&mut columns))
}

pub fn parse_select(sql: &str) -> Result<Select, ParseError> {
    parse_select_tokens(tokenize(sql)?)
}

fn parse_select_tokens(tokens: Vec<Token>) -> Result<Select, ParseError> {
    let mut parser = Parser { tokens, index: 0 };
    if !parser.keyword("SELECT") {
        return Err(ParseError("only SELECT statements are supported".into()));
    }
    let mut distinct = parser.keyword("DISTINCT");
    let mut literal_alias = None;

    let multiple = multi_aggregate::parse_multiple(&mut parser)?;
    let projection = if let Some(items) = multiple {
        Projection::MultipleAggregates(items)
    } else if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("LENGTH"))
        && parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
        && matches!(parser.tokens.get(parser.index + 2), Some(Token::Word(name)) if !name.eq_ignore_ascii_case("NULL"))
        && parser.tokens.get(parser.index + 3) == Some(&Token::RParen)
    {
        parser.keyword("LENGTH");
        parser.take(Token::LParen);
        let column = parser.word()?;
        parser.take(Token::RParen);
        Projection::StringLength { column }
    } else if parser.take(Token::Star) {
        Projection::All
    } else if matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
        && parser.keyword("COUNT")
    {
        if !parser.take(Token::LParen) {
            return Err(ParseError("expected COUNT() or COUNT(*)".into()));
        }
        if parser.keyword("DISTINCT") {
            let column = parser.word()?;
            if !parser.take(Token::RParen) {
                return Err(ParseError(
                    "expected ')' after COUNT(DISTINCT column)".into(),
                ));
            }
            Projection::CountDistinct { column }
        } else if parser.take(Token::RParen) {
            Projection::Count
        } else if parser.take(Token::Star) {
            if !parser.take(Token::RParen) {
                return Err(ParseError("expected ')' after COUNT(*)".into()));
            }
            Projection::Count
        } else {
            let column = parser.word()?;
            if !parser.take(Token::RParen) {
                return Err(ParseError("expected ')' after COUNT(column)".into()));
            }
            Projection::CountColumn { column }
        }
    } else if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("UNIQSTATE"))
        && parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
    {
        parser.keyword("UNIQSTATE");
        parser.take(Token::LParen);
        let column = parser.word()?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after uniqState(column)".into()));
        }
        Projection::ApproxDistinctState { column }
    } else if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("UNIQ"))
        && parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
    {
        parser.keyword("UNIQ");
        parser.take(Token::LParen);
        let column = parser.word()?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after uniq(column)".into()));
        }
        Projection::ApproxDistinct { column }
    } else if matches!(parser.tokens.get(parser.index), Some(Token::Word(name)) if name.eq_ignore_ascii_case("HISTOGRAM"))
        && parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
    {
        parser.keyword("HISTOGRAM");
        parser.take(Token::LParen);
        let bins = parser.number()?.parse::<usize>()
            .map_err(|_| ParseError("histogram bins must be an integer".into()))?;
        if !(1..=256).contains(&bins) {
            return Err(ParseError("histogram requires between 1 and 256 bins".into()));
        }
        if !parser.take(Token::RParen) || !parser.take(Token::LParen) {
            return Err(ParseError("histogram requires histogram(bins)(column)".into()));
        }
        let column = parser.word()?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after histogram column".into()));
        }
        Projection::Histogram { column, bins }
    } else if matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
        && !matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if scalar::is_function(word) || text_scalar::is_function(word))
    {
        let name = parser.word()?;
        let function = match name.to_ascii_uppercase().as_str() {
            "GROUPBITXOR" => AggregateFunction::GroupBitXor,
            "GROUPBITOR" => AggregateFunction::GroupBitOr,
            "GROUPBITAND" => AggregateFunction::GroupBitAnd,
            "ANYLAST" => AggregateFunction::AnyLast,
            "ANY" => AggregateFunction::Any,
            "COUNTIF" => AggregateFunction::CountIf,
            "SUM" => AggregateFunction::Sum,
            "PRODUCT" => AggregateFunction::Product,
            "AVG" => AggregateFunction::Avg,
            "SUMSQUARES" => AggregateFunction::SumSquares,
            "MIN" => AggregateFunction::Min,
            "RANGE" => AggregateFunction::Range,
            "MAX" => AggregateFunction::Max,
            "RMS" => AggregateFunction::Rms,
            "VARPOP" => AggregateFunction::VarPop,
            "STDDEVPOP" => AggregateFunction::StddevPop,
            "VARSAMP" => AggregateFunction::VarSamp,
            "STDDEVSAMP" | "STDDEV" => AggregateFunction::StddevSamp,
            "MEDIAN" => AggregateFunction::Median,
            _ => return Err(ParseError(format!("unsupported aggregate: {name}"))),
        };
        parser.take(Token::LParen);
        let column = parser.word()?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after aggregate column".into()));
        }
        Projection::Aggregate { function, column }
    } else if matches!(
        parser.tokens.get(parser.index),
        Some(Token::Number(_) | Token::String(_) | Token::LParen | Token::Plus | Token::Minus)
    ) || matches!(
        parser.tokens.get(parser.index),
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL") || scalar::is_function(word) || text_scalar::is_function(word)
    ) {
        let mut columns = Vec::new();
        let is_text_scalar = matches!(
            parser.tokens.get(parser.index),
            Some(Token::Word(name)) if text_scalar::is_function(name)
        );
        loop {
            if columns.len() >= 64 {
                return Err(ParseError("SELECT supports at most 64 constants".into()));
            }
            let value = match parser.tokens.get(parser.index) {
                Some(Token::Number(_) | Token::LParen | Token::Plus | Token::Minus) => {
                    ConstantValue::Integer(scalar::parse(&mut parser)?)
                }
                Some(Token::Word(name)) if scalar::is_function(name) => {
                    ConstantValue::Integer(scalar::parse(&mut parser)?)
                }
                Some(Token::Word(name)) if text_scalar::is_function(name) => {
                    text_scalar::parse(&mut parser)?
                }
                Some(Token::String(value)) => {
                    let value = value.clone();
                    parser.index += 1;
                    ConstantValue::Text(value)
                }
                Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL") => {
                    parser.index += 1;
                    ConstantValue::Null
                }
                _ => return Err(ParseError("expected a constant after comma".into())),
            };
            let alias = if parser.keyword("AS") {
                Some(parser.word()?)
            } else {
                None
            };
            columns.push(Constant { value, alias });
            if !parser.take(Token::Comma) {
                break;
            }
        }
        if is_text_scalar {
            Projection::Constants(columns)
        } else if let [
            Constant {
                value: ConstantValue::Integer(value),
                alias,
            },
        ] = columns.as_slice()
        {
            literal_alias = alias.clone();
            Projection::Literal(*value)
        } else {
            Projection::Constants(columns)
        }
    } else {
        let mut columns = vec![parser.word()?];
        let mut grouped_count = false;
        let mut grouped_distinct = None;
        let mut grouped_aggregate = None;
        let mut grouped_aggregates = None;
        while parser.take(Token::Comma) {
            if matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if word.eq_ignore_ascii_case("COUNT"))
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
                && !matches!(
                    parser.tokens.get(parser.index + 2),
                    Some(Token::Word(word)) if !word.eq_ignore_ascii_case("DISTINCT")
                )
            {
                parser.keyword("COUNT");
                if !parser.take(Token::LParen) {
                    return Err(ParseError("expected COUNT() or COUNT(*)".into()));
                }
                if parser.keyword("DISTINCT") {
                    if columns.len() > 8 {
                        return Err(ParseError("GROUP BY supports at most 8 keys".into()));
                    }
                    let value_column = parser.word()?;
                    if !parser.take(Token::RParen) {
                        return Err(ParseError(
                            "expected ')' after COUNT(DISTINCT column)".into(),
                        ));
                    }
                    grouped_distinct = Some(value_column);
                } else {
                    if !parser.take(Token::RParen)
                        && !(parser.take(Token::Star) && parser.take(Token::RParen))
                    {
                        return Err(ParseError("expected COUNT() or COUNT(*)".into()));
                    }
                    grouped_count = true;
                }
                break;
            }
            if parser.tokens.get(parser.index + 1) == Some(&Token::LParen) {
                let function = match parser.tokens.get(parser.index) {
                    Some(Token::Word(name)) => match name.to_ascii_uppercase().as_str() {
                        "GROUPBITXOR" => Some(AggregateFunction::GroupBitXor),
                        "GROUPBITOR" => Some(AggregateFunction::GroupBitOr),
                        "GROUPBITAND" => Some(AggregateFunction::GroupBitAnd),
                        "ANYLAST" => Some(AggregateFunction::AnyLast),
                        "ANY" => Some(AggregateFunction::Any),
                        "COUNTIF" => Some(AggregateFunction::CountIf),
                        "COUNT" => Some(AggregateFunction::Count),
                        "SUM" | "SUMIF" => Some(AggregateFunction::Sum),
                        "PRODUCT" => Some(AggregateFunction::Product),
                        "AVG" | "AVGIF" => Some(AggregateFunction::Avg),
                        "SUMSQUARES" => Some(AggregateFunction::SumSquares),
                        "MIN" | "MINIF" => Some(AggregateFunction::Min),
                        "RANGE" => Some(AggregateFunction::Range),
                        "MAX" | "MAXIF" => Some(AggregateFunction::Max),
                        "RMS" => Some(AggregateFunction::Rms),
                        "VARPOP" => Some(AggregateFunction::VarPop),
                        "STDDEVPOP" => Some(AggregateFunction::StddevPop),
                        "VARSAMP" => Some(AggregateFunction::VarSamp),
                        "STDDEVSAMP" | "STDDEV" => Some(AggregateFunction::StddevSamp),
                        "MEDIAN" => Some(AggregateFunction::Median),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(function) = function {
                    let conditional = matches!(parser.tokens.get(parser.index),
                        Some(Token::Word(name)) if matches!(name.to_ascii_uppercase().as_str(),
                            "SUMIF" | "AVGIF" | "MINIF" | "MAXIF"));
                    parser.index += 1;
                    parser.take(Token::LParen);
                    let value_column = parser.word()?;
                    let filter = if conditional {
                        if !parser.take(Token::Comma) {
                            return Err(ParseError("conditional aggregate requires filter column".into()));
                        }
                        Some(parser.word()?)
                    } else {
                        None
                    };
                    if !parser.take(Token::RParen) {
                        return Err(ParseError("expected ')' after grouped aggregate".into()));
                    }
                    let mut items = vec![(function, value_column)];
                    let mut filters = vec![filter];
                    let mut aliases = vec![if parser.keyword("AS") {
                        Some(parser.word()?)
                    } else {
                        None
                    }];
                    while parser.take(Token::Comma) {
                        if items.len() >= 16 {
                            return Err(ParseError(
                                "grouped SELECT supports at most 16 numeric aggregates".into(),
                            ));
                        }
                        let name = parser.word()?;
                        let conditional = matches!(name.to_ascii_uppercase().as_str(),
                            "SUMIF" | "AVGIF" | "MINIF" | "MAXIF");
                        let function = match name.to_ascii_uppercase().as_str() {
                            "MIN" | "MINIF" => AggregateFunction::Min,
                            "MAX" | "MAXIF" => AggregateFunction::Max,
                            "AVG" | "AVGIF" => AggregateFunction::Avg,
                            "SUM" | "SUMIF" => AggregateFunction::Sum,
                            "COUNT" => AggregateFunction::Count,
                            "COUNTIF" => AggregateFunction::CountIf,
                            "PRODUCT" => AggregateFunction::Product,
                            "SUMSQUARES" => AggregateFunction::SumSquares,
                            "RANGE" => AggregateFunction::Range,
                            "RMS" => AggregateFunction::Rms,
                            "VARPOP" => AggregateFunction::VarPop,
                            "VARSAMP" => AggregateFunction::VarSamp,
                            "STDDEVPOP" => AggregateFunction::StddevPop,
                            "STDDEVSAMP" => AggregateFunction::StddevSamp,
                            "MEDIAN" => AggregateFunction::Median,
                            "ANY" => AggregateFunction::Any,
                            "ANYLAST" => AggregateFunction::AnyLast,
                            "GROUPBITAND" => AggregateFunction::GroupBitAnd,
                            "GROUPBITOR" => AggregateFunction::GroupBitOr,
                            "GROUPBITXOR" => AggregateFunction::GroupBitXor,
                            _ => {
                                return Err(ParseError(format!(
                                    "unsupported grouped aggregate: {name}"
                                )));
                            }
                        };
                        if !parser.take(Token::LParen) {
                            return Err(ParseError("expected '(' after aggregate name".into()));
                        }
                        let value_column = parser.word()?;
                        let filter = if conditional {
                            if !parser.take(Token::Comma) {
                                return Err(ParseError("conditional aggregate requires filter column".into()));
                            }
                            Some(parser.word()?)
                        } else {
                            None
                        };
                        if !parser.take(Token::RParen) {
                            return Err(ParseError("expected ')' after grouped aggregate".into()));
                        }
                        items.push((function, value_column));
                        filters.push(filter);
                        aliases.push(if parser.keyword("AS") {
                            Some(parser.word()?)
                        } else {
                            None
                        });
                    }
                    let mut seen = std::collections::HashSet::new();
                    for alias in aliases.iter().flatten() {
                        if columns.iter().any(|column| column.eq_ignore_ascii_case(alias))
                            || !seen.insert(alias.to_ascii_lowercase())
                        {
                            return Err(ParseError("duplicate or ambiguous aggregate alias".into()));
                        }
                    }
                    if items.len() == 1 && columns.len() == 1
                        && aliases[0].is_none() && filters[0].is_none() {
                        grouped_aggregate = items.pop();
                    } else {
                        grouped_aggregates = Some((items, aliases, filters));
                    }
                    break;
                }
            }
            columns.push(parser.word()?);
        }
        if grouped_count {
            if columns.len() > 8 {
                return Err(ParseError("GROUP BY supports at most 8 keys".into()));
            }
            if columns.len() == 1 {
                Projection::GroupedCount {
                    column: columns.remove(0),
                }
            } else {
                Projection::GroupedMultiCount { columns }
            }
        } else if let Some(value_column) = grouped_distinct {
            if columns.len() == 1 {
                Projection::GroupedDistinctCount {
                    column: columns.remove(0),
                    value_column,
                }
            } else {
                Projection::GroupedMultiDistinctCount { columns, value_column }
            }
        } else if let Some((function, value_column)) = grouped_aggregate {
            Projection::GroupedAggregate {
                column: columns.remove(0),
                function,
                value_column,
            }
        } else if let Some((items, aliases, filters)) = grouped_aggregates {
            if columns.len() == 1 {
                Projection::GroupedAggregates {
                    column: columns.remove(0),
                    items,
                    aliases,
                    filters,
                }
            } else {
                if columns.len() > 8 {
                    return Err(ParseError("GROUP BY supports at most 8 keys".into()));
                }
                Projection::GroupedMultiAggregates { columns, items, aliases, filters }
            }
        } else {
            Projection::Columns(columns)
        }
    };

    if distinct && !matches!(projection, Projection::All | Projection::Columns(_)) {
        return Err(ParseError("DISTINCT supports only row projections".into()));
    }
    let rename = if parser.keyword("RENAME") {
        if !matches!(projection, Projection::All) {
            return Err(ParseError("RENAME requires SELECT *".into()));
        }
        let source = parser.word()?;
        if !parser.keyword("AS") {
            return Err(ParseError("RENAME requires AS".into()));
        }
        Some((source, parser.word()?))
    } else {
        None
    };

    let alias = if let Some(alias) = literal_alias {
        Some(alias)
    } else if parser.keyword("AS") {
        let single_result = matches!(
            &projection,
            Projection::Literal(_)
                | Projection::Count
                | Projection::CountColumn { .. }
                | Projection::CountDistinct { .. }
                | Projection::ApproxDistinct { .. }
                | Projection::Histogram { .. }
                | Projection::ApproxDistinctState { .. }
                | Projection::Aggregate { .. }
                | Projection::GroupedCount { .. }
                | Projection::GroupedDistinctCount { .. }
                | Projection::GroupedMultiCount { .. }
                | Projection::StringLength { .. }
        ) || matches!(&projection, Projection::Columns(columns) if columns.len() == 1);
        if !single_result {
            return Err(ParseError("AS supports only a single result column".into()));
        }
        Some(parser.word()?)
    } else {
        None
    };

    let source = if parser.keyword("FROM") {
        let name = parser.word()?;
        if name.eq_ignore_ascii_case("numbers") {
            if !parser.take(Token::LParen) {
                return Err(ParseError("numbers requires a row count".into()));
            }
            let count = parser.number()?;
            if !count.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(ParseError("numbers row count must be non-negative".into()));
            }
            let first = count
                .parse::<u64>()
                .map_err(|_| ParseError("numbers argument is too large".into()))?;
            let source = if parser.take(Token::Comma) {
                let count = parser.number()?;
                if !count.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(ParseError("numbers row count must be non-negative".into()));
                }
                let count = count
                    .parse::<u64>()
                    .map_err(|_| ParseError("numbers row count is too large".into()))?;
                if count > 1_000_000_000 {
                    return Err(ParseError(
                        "numbers supports at most 1000000000 rows".into(),
                    ));
                }
                if parser.take(Token::Comma) {
                    let step = parser
                        .number()?
                        .parse::<u64>()
                        .map_err(|_| ParseError("numbers step must be a positive UInt64".into()))?;
                    if step == 0 {
                        return Err(ParseError("numbers step must be positive".into()));
                    }
                    Source::NumbersStepped {
                        start: first,
                        length: count,
                        step,
                    }
                } else {
                    Source::NumbersRange {
                        start: first,
                        count,
                    }
                }
            } else {
                if first > 1_000_000_000 {
                    return Err(ParseError(
                        "numbers supports at most 1000000000 rows".into(),
                    ));
                }
                Source::Numbers(first)
            };
            let last_offset = match &source {
                Source::NumbersRange { count, .. } if *count > 0 => Some(*count - 1),
                Source::NumbersStepped { length, step, .. } if *length > 0 => {
                    Some(((*length - 1) / *step) * *step)
                }
                _ => None,
            };
            if last_offset.is_some_and(|offset| first.checked_add(offset).is_none()) {
                return Err(ParseError("numbers range overflows UInt64".into()));
            }
            if !parser.take(Token::RParen) {
                return Err(ParseError("expected ')' after numbers row count".into()));
            }
            Some(source)
        } else if name.eq_ignore_ascii_case("generate_series") {
            if !parser.take(Token::LParen) {
                return Err(ParseError(
                    "generate_series requires (start, stop[, step])".into(),
                ));
            }
            let parse_argument = |parser: &mut Parser, label: &str| -> Result<u64, ParseError> {
                let value = parser.number()?;
                if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(ParseError(format!(
                        "generate_series {label} must be a non-negative UInt64"
                    )));
                }
                value
                    .parse::<u64>()
                    .map_err(|_| ParseError(format!("generate_series {label} is too large")))
            };
            let start = parse_argument(&mut parser, "start")?;
            if !parser.take(Token::Comma) {
                return Err(ParseError(
                    "generate_series requires a stop argument".into(),
                ));
            }
            let stop = parse_argument(&mut parser, "stop")?;
            let step = if parser.take(Token::Comma) {
                parse_argument(&mut parser, "step")?
            } else {
                1
            };
            if step == 0 {
                return Err(ParseError("generate_series step must be positive".into()));
            }
            if !parser.take(Token::RParen) {
                return Err(ParseError(
                    "expected ')' after generate_series arguments".into(),
                ));
            }
            let count = if stop < start {
                0
            } else {
                ((stop - start) / step).checked_add(1).ok_or_else(|| {
                    ParseError("generate_series supports at most 10000000 rows".into())
                })?
            };
            if count > 10_000_000 {
                return Err(ParseError(
                    "generate_series supports at most 10000000 rows".into(),
                ));
            }
            Some(Source::GenerateSeries { start, count, step })
        } else {
            Some(Source::Table(name))
        }
    } else {
        None
    };
    if source.is_none()
        && !matches!(
            projection,
            Projection::Literal(_) | Projection::Constants(_)
        )
    {
        return Err(ParseError("a table source is required".into()));
    }
    if source.is_some()
        && matches!(
            projection,
            Projection::Literal(_) | Projection::Constants(_)
        )
    {
        return Err(ParseError("literal SELECT does not support FROM".into()));
    }

    // ClickHouse evaluates PREWHERE as an additional filtering stage. Lowering
    // PREWHERE AND WHERE into the shared bounded DNF preserves OR precedence
    // and uses the same typed predicate executor for all table sources.
    let prewhere = if parser.keyword("PREWHERE") {
        if source.is_none() {
            return Err(ParseError("PREWHERE requires a table source".into()));
        }
        Some(where_parser::parse_where(&mut parser)?)
    } else {
        None
    };
    let where_clause = if parser.keyword("WHERE") {
        if source.is_none() {
            return Err(ParseError("WHERE requires a table source".into()));
        }
        Some(where_parser::parse_where(&mut parser)?)
    } else {
        None
    };
    let branches = match (prewhere, where_clause) {
        (Some(prewhere), Some(where_clause)) => {
            where_parser::combine_and(prewhere, where_clause)?
        }
        (Some(branches), None) | (None, Some(branches)) => branches,
        (None, None) => Vec::new(),
    };
    let mut branches = branches.into_iter();
    let predicates = branches.next().unwrap_or_default();
    let or_predicates = branches.collect();

    if parser.keyword("GROUP") {
        if !parser.keyword("BY") {
            return Err(ParseError("expected BY after GROUP".into()));
        }
        // ClickHouse GROUP BY ALL groups by every non-aggregate output
        // expression. In this bounded SELECT dialect those expressions are
        // either the explicit grouping keys or all plain projected columns.
        if parser.keyword("ALL") {
            match &projection {
                Projection::GroupedCount { .. }
                | Projection::GroupedMultiCount { .. }
                | Projection::GroupedDistinctCount { .. }
                | Projection::GroupedMultiDistinctCount { .. }
                | Projection::GroupedAggregate { .. }
                | Projection::GroupedAggregates { .. }
                | Projection::GroupedMultiAggregates { .. } => {}
                Projection::All | Projection::Columns(_) => distinct = true,
                Projection::Count
                | Projection::CountColumn { .. }
                | Projection::CountDistinct { .. }
                | Projection::ApproxDistinct { .. }
                | Projection::ApproxDistinctState { .. }
                | Projection::Histogram { .. }
                | Projection::Aggregate { .. }
                | Projection::MultipleAggregates(_) => {}
                _ => return Err(ParseError(
                    "GROUP BY ALL requires columns or aggregate projections".into(),
                )),
            }
        } else {
            let mut group_columns = Vec::new();
            loop {
                if group_columns.len() >= 8 {
                    return Err(ParseError("GROUP BY supports at most 8 keys".into()));
                }
                let column = if matches!(parser.tokens.get(parser.index), Some(Token::Number(_))) {
                    let position = parser.number()?.parse::<usize>()
                        .map_err(|_| ParseError("GROUP BY position is too large".into()))?;
                    match &projection {
                        Projection::GroupedCount { column }
                        | Projection::GroupedDistinctCount { column, .. }
                        | Projection::GroupedAggregate { column, .. }
                        | Projection::GroupedAggregates { column, .. } if position == 1 => column.clone(),
                        Projection::GroupedMultiCount { columns }
                        | Projection::GroupedMultiAggregates { columns, .. }
                        | Projection::GroupedMultiDistinctCount { columns, .. } => position
                            .checked_sub(1).and_then(|index| columns.get(index)).cloned()
                            .ok_or_else(|| ParseError("GROUP BY position is out of range".into()))?,
                        _ => return Err(ParseError("GROUP BY position is out of range".into())),
                    }
                } else {
                    parser.word()?
                };
                group_columns.push(column);
                if !parser.take(Token::Comma) {
                    break;
                }
            }
            match &projection {
                Projection::GroupedMultiCount { columns } if columns == &group_columns => {}
                Projection::GroupedMultiAggregates { columns, .. }
                | Projection::GroupedMultiDistinctCount { columns, .. }
                    if columns == &group_columns => {}
                Projection::GroupedCount { column }
                | Projection::GroupedDistinctCount { column, .. }
                | Projection::GroupedAggregate { column, .. }
                | Projection::GroupedAggregates { column, .. }
                    if group_columns.len() == 1 && column == &group_columns[0] => {}
                Projection::GroupedCount { .. }
                | Projection::GroupedDistinctCount { .. }
                | Projection::GroupedMultiDistinctCount { .. }
                | Projection::GroupedMultiCount { .. }
                | Projection::GroupedAggregate { .. }
                | Projection::GroupedAggregates { .. }
                | Projection::GroupedMultiAggregates { .. } => {
                    return Err(ParseError(
                        "GROUP BY must match selected grouping keys".into(),
                    ));
                }
                _ => {
                    return Err(ParseError(
                        "GROUP BY requires SELECT key, aggregate(column)".into(),
                    ));
                }
            }
        }
    } else if matches!(
        projection,
        Projection::GroupedCount { .. }
            | Projection::GroupedDistinctCount { .. }
            | Projection::GroupedMultiDistinctCount { .. }
            | Projection::GroupedMultiCount { .. }
            | Projection::GroupedAggregate { .. }
            | Projection::GroupedAggregates { .. }
            | Projection::GroupedMultiAggregates { .. }
    ) {
        return Err(ParseError("grouped aggregates require GROUP BY".into()));
    }

    let having = if parser.keyword("HAVING") {
        let mut aggregate_index = None;
        if matches!(
            projection,
            Projection::GroupedCount { .. } | Projection::GroupedMultiCount { .. }
        ) {
            let using_alias = alias.as_ref().is_some_and(|value| {
                matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if word == value)
            });
            if using_alias {
                parser.index += 1;
            } else {
                if !parser.keyword("COUNT") || !parser.take(Token::LParen) {
                    return Err(ParseError("HAVING requires COUNT(*)".into()));
                }
                if !parser.take(Token::RParen)
                    && !(parser.take(Token::Star) && parser.take(Token::RParen))
                {
                    return Err(ParseError("HAVING requires COUNT() or COUNT(*)".into()));
                }
            }
        } else if let Projection::GroupedAggregate {
            function,
            value_column,
            ..
        } = &projection
        {
            // The grouped state can evaluate HAVING on all numeric metrics.
            // Invalid NULL results are removed by SQL HAVING semantics.
            if !matches!(
                function,
                AggregateFunction::Sum
                    | AggregateFunction::Min
                    | AggregateFunction::Max
                    | AggregateFunction::Avg
                    | AggregateFunction::Count
                    | AggregateFunction::CountIf
                    | AggregateFunction::Product
                    | AggregateFunction::SumSquares
                    | AggregateFunction::Range
                    | AggregateFunction::Rms
                    | AggregateFunction::VarPop
                    | AggregateFunction::VarSamp
                    | AggregateFunction::StddevPop
                    | AggregateFunction::StddevSamp
                    | AggregateFunction::Median
                    | AggregateFunction::Any
                    | AggregateFunction::AnyLast
                    | AggregateFunction::GroupBitAnd
                    | AggregateFunction::GroupBitOr
                    | AggregateFunction::GroupBitXor
            ) {
                return Err(ParseError("HAVING does not support this aggregate".into()));
            }
            let name = parser.word()?;
            if !name.eq_ignore_ascii_case(function.name())
                || !parser.take(Token::LParen)
                || parser.word()? != *value_column
                || !parser.take(Token::RParen)
            {
                return Err(ParseError(
                    "HAVING must match the selected aggregate".into(),
                ));
            }
        } else if let Projection::GroupedAggregates { items, aliases, .. }
        | Projection::GroupedMultiAggregates { items, aliases, .. } = &projection {
            let name = parser.word()?;
            // HAVING uses the same aggregate alias resolution as ORDER BY.
            // Only a projected metric can be filtered, so an alias never
            // selects a different or previously uncomputed aggregate.
            let selected = if parser.take(Token::LParen) {
                let column = parser.word()?;
                if !parser.take(Token::RParen) {
                    return Err(ParseError("expected ')' after HAVING metric".into()));
                }
                items.iter().position(|(function, argument)| {
                    function.name().eq_ignore_ascii_case(&name) && argument == &column
                })
            } else {
                aliases.iter().position(|alias| alias.as_ref()
                    .is_some_and(|alias| alias.eq_ignore_ascii_case(&name)))
            };
            aggregate_index = Some(selected.filter(|&index| {
                matches!(items[index].0,
                    AggregateFunction::Sum | AggregateFunction::Avg
                    | AggregateFunction::Min | AggregateFunction::Max
                    | AggregateFunction::Count)
            }).ok_or_else(|| ParseError("HAVING metric must match SELECT".into()))?);
        } else if let Projection::GroupedDistinctCount { value_column, .. }
        | Projection::GroupedMultiDistinctCount { value_column, .. } = &projection {
            if !parser.keyword("COUNT")
                || !parser.take(Token::LParen)
                || !parser.keyword("DISTINCT")
                || parser.word()? != *value_column
                || !parser.take(Token::RParen)
            {
                return Err(ParseError(
                    "HAVING must match the selected COUNT(DISTINCT column)".into(),
                ));
            }
        } else {
            return Err(ParseError("HAVING requires a grouped aggregate".into()));
        }
        let operator = parser.operator()?;
        let value = parser
            .number()?
            .parse::<u64>()
            .map_err(|_| ParseError("HAVING threshold must be a UInt64".into()))?;
        Some(Having { operator, value, aggregate_index })
    } else {
        None
    };

    let mut order_by_extra = Vec::new();
    let mut order_by = if parser.keyword("ORDER") {
        if !parser.keyword("BY") {
            return Err(ParseError("expected BY after ORDER".into()));
        }
        if parser.keyword("ALL") {
            let columns = order_by_all_columns(&projection)?;
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
            let mut terms = columns.into_iter().map(|column| OrderBy {
                column, descending, nulls_first,
            });
            let first = terms.next().expect("ORDER BY ALL has columns");
            order_by_extra.extend(terms);
            Some(first)
        } else {
            let first = parser.order_term(&projection)?;
            while parser.take(Token::Comma) {
                if order_by_extra.len() >= 7 {
                    return Err(ParseError("ORDER BY supports at most 8 columns".into()));
                }
                order_by_extra.push(parser.order_term(&projection)?);
            }
            Some(first)
        }
    } else {
        None
    };
    if matches!(
        projection,
        Projection::GroupedCount { .. }
            | Projection::GroupedDistinctCount { .. }
            | Projection::GroupedMultiDistinctCount { .. }
            | Projection::GroupedMultiCount { .. }
    ) {
        if let Some(count_alias) = alias.as_deref() {
            for term in order_by.iter_mut().chain(order_by_extra.iter_mut()) {
                if term.column == count_alias {
                    term.column = "count".into();
                }
            }
        }
    }
    if order_by.is_some()
        && matches!(
            projection,
            Projection::Count
                | Projection::CountColumn { .. }
                | Projection::CountDistinct { .. }
                | Projection::ApproxDistinct { .. }
                | Projection::Histogram { .. }
                | Projection::ApproxDistinctState { .. }
                | Projection::Aggregate { .. }
                | Projection::MultipleAggregates(_)
                | Projection::StringLength { .. }
                | Projection::Literal(_)
        )
    {
        return Err(ParseError("ORDER BY requires column projections".into()));
    }

    if let Projection::GroupedCount { column } | Projection::GroupedDistinctCount { column, .. } =
        &projection
    {
        if order_by
            .iter()
            .chain(order_by_extra.iter())
            .any(|order| &order.column != column && order.column != "count")
        {
            return Err(ParseError("grouped ORDER BY must use the group key".into()));
        }
    }
    if let Projection::GroupedMultiCount { columns }
        | Projection::GroupedMultiDistinctCount { columns, .. } = &projection {
        if order_by
            .iter()
            .chain(order_by_extra.iter())
            .any(|order| !columns.contains(&order.column) && order.column != "count")
        {
            return Err(ParseError(
                "grouped ORDER BY must use the group keys or count".into(),
            ));
        }
    }
    if let Projection::GroupedAggregate { column, .. } = &projection {
        if order_by
            .iter()
            .chain(order_by_extra.iter())
            .any(|order| &order.column != column && order.column != "#aggregate")
        {
            return Err(ParseError(
                "grouped numeric ORDER BY must use the group key or aggregate".into(),
            ));
        }
    }
    if let Projection::GroupedAggregates { column, items, .. } = &projection {
        let metric = |name: &str| name.strip_prefix("#aggregate")
            .and_then(|number| number.parse::<usize>().ok())
            .is_some_and(|index| index < items.len());
        if order_by.iter().chain(order_by_extra.iter())
            .any(|order| &order.column != column && !metric(&order.column))
        {
            return Err(ParseError(
                "grouped numeric ORDER BY must use the group key or a selected metric".into(),
            ));
        }
    }

    if let Projection::GroupedMultiAggregates { columns, items, .. } = &projection {
        let metric = |name: &str| name.strip_prefix("#aggregate")
            .and_then(|number| number.parse::<usize>().ok())
            .is_some_and(|index| index < items.len());
        if order_by.iter().chain(order_by_extra.iter())
            .any(|order| !columns.contains(&order.column) && !metric(&order.column))
        {
            return Err(ParseError(
                "grouped numeric ORDER BY must use the group keys or selected metrics".into(),
            ));
        }
    }

    let mut comma_offset = None;
    let mut limit = if parser.keyword("LIMIT") {
        let value = parser.number()?;
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError("LIMIT must be a non-negative integer".into()));
        }
        let first = value
            .parse::<usize>()
            .map_err(|_| ParseError("LIMIT is too large".into()))?;
        if parser.take(Token::Comma) {
            let value = parser.number()?;
            if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(ParseError("LIMIT must be a non-negative integer".into()));
            }
            comma_offset = Some(first);
            Some(
                value
                    .parse::<usize>()
                    .map_err(|_| ParseError("LIMIT is too large".into()))?,
            )
        } else {
            Some(first)
        }
    } else {
        None
    };

    // Accept both ClickHouse LIMIT pagination and SQL-standard OFFSET/FETCH.
    let offset = if parser.keyword("OFFSET") {
        if comma_offset.is_some() {
            return Err(ParseError(
                "cannot combine LIMIT comma syntax with OFFSET".into(),
            ));
        }
        let value = parser.number()?;
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError("OFFSET must be a non-negative integer".into()));
        }
        let offset = value
            .parse::<usize>()
            .map_err(|_| ParseError("OFFSET is too large".into()))?;
        if !parser.keyword("ROWS") {
            parser.keyword("ROW");
        }
        Some(offset)
    } else {
        comma_offset
    };

    if parser.keyword("FETCH") {
        if limit.is_some() {
            return Err(ParseError("cannot combine FETCH with LIMIT".into()));
        }
        if !parser.keyword("FIRST") && !parser.keyword("NEXT") {
            return Err(ParseError("FETCH requires FIRST or NEXT".into()));
        }
        let value = parser.number()?;
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError(
                "FETCH count must be a non-negative integer".into(),
            ));
        }
        let count = value
            .parse::<usize>()
            .map_err(|_| ParseError("FETCH count is too large".into()))?;
        if !parser.keyword("ROWS") && !parser.keyword("ROW") {
            return Err(ParseError("FETCH requires ROW or ROWS".into()));
        }
        if !parser.keyword("ONLY") {
            return Err(ParseError("FETCH requires ONLY".into()));
        }
        limit = Some(count);
    }
    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError(
            "unexpected tokens after SELECT statement".into(),
        ));
    }
    Ok(Select {
        projection,
        alias,
        rename,
        distinct,
        source,
        predicates,
        or_predicates,
        order_by,
        order_by_extra,
        having,
        limit,
        offset,
    })
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
            let canonical = input.replace("stddev(", "stddevSamp(").replace("STDDEV(", "stddevSamp(");
            assert_eq!(parse_select(input).unwrap().projection, parse_select(&canonical).unwrap().projection);
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
            ("SELECT count(*) FROM lineitem WHERE l_shipdate <= DATE '1998-12-01' - INTERVAL '90' DAY", "1998-09-02"),
            ("SELECT count(*) FROM orders WHERE o_orderdate < DATE '1994-01-01' + INTERVAL '3' MONTH", "1994-04-01"),
            ("SELECT count(*) FROM lineitem WHERE l_shipdate < DATE '1994-01-01' + INTERVAL '1' YEAR", "1995-01-01"),
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
