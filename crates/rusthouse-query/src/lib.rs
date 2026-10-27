mod copy;
mod ddl;
mod delete;
mod execute;
mod explain;
mod insert;
mod insert_select;
mod join;
mod lexer;
mod scalar;
mod update;
pub use copy::{CopyFormat, CopyFrom, parse_copy_from};
pub use ddl::{ColumnDef, Ddl, parse_ddl};
pub use delete::parse_delete;
pub use execute::{ExecutionError, RowPredicate, execute, execute_join};
pub use explain::explain_select;
pub use insert::{Insert, parse_insert};
pub use insert_select::{InsertSelect, parse_insert_select};
pub use join::{JoinColumn, JoinSelect, JoinType, explain_join, parse_join};
pub use update::{Update, parse_update};

use std::error::Error;
use std::fmt;

use lexer::{Token, tokenize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateFunction {
    Sum,
    Avg,
    Min,
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
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Min => "min",
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
    GroupedCount {
        column: String,
    },
    GroupedAggregate {
        column: String,
        function: AggregateFunction,
        value_column: String,
    },
    Aggregate {
        function: AggregateFunction,
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
            Some(Token::Word(value)) => {
                self.index += 1;
                Ok(value.clone())
            }
            _ => Err(ParseError("expected an identifier".into())),
        }
    }

    fn literal(&mut self) -> Result<String, ParseError> {
        match self.tokens.get(self.index) {
            Some(Token::Number(value)) | Some(Token::String(value)) => {
                self.index += 1;
                Ok(value.clone())
            }
            _ => Err(ParseError("expected a SQL literal".into())),
        }
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
                Projection::GroupedCount { column } => match position {
                    1 => column.clone(),
                    2 => "count".into(),
                    _ => return Err(ParseError("ORDER BY position is out of range".into())),
                },
                Projection::GroupedAggregate { column, .. } => match position {
                    1 => column.clone(),
                    _ => {
                        return Err(ParseError(
                            "grouped numeric aggregates only support ORDER BY the group key".into(),
                        ));
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
        if matches!(projection, Projection::GroupedCount { .. })
            && column.eq_ignore_ascii_case("count")
            && self.take(Token::LParen)
            && !self.take(Token::RParen)
            && !(self.take(Token::Star) && self.take(Token::RParen))
        {
            return Err(ParseError(
                "expected COUNT() or COUNT(*) in ORDER BY".into(),
            ));
        }
        let column = if matches!(projection, Projection::GroupedCount { .. })
            && column.eq_ignore_ascii_case("count")
        {
            "count".to_string()
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

pub fn parse_select(sql: &str) -> Result<Select, ParseError> {
    parse_select_tokens(tokenize(sql)?)
}

fn parse_select_tokens(tokens: Vec<Token>) -> Result<Select, ParseError> {
    let mut parser = Parser { tokens, index: 0 };
    if !parser.keyword("SELECT") {
        return Err(ParseError("only SELECT statements are supported".into()));
    }
    let distinct = parser.keyword("DISTINCT");
    let mut literal_alias = None;

    let projection = if parser.take(Token::Star) {
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
    } else if matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen))
        && !matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if scalar::is_function(word))
    {
        let name = parser.word()?;
        let function = match name.to_ascii_uppercase().as_str() {
            "SUM" => AggregateFunction::Sum,
            "AVG" => AggregateFunction::Avg,
            "MIN" => AggregateFunction::Min,
            "MAX" => AggregateFunction::Max,
            "RMS" => AggregateFunction::Rms,
            "VARPOP" => AggregateFunction::VarPop,
            "STDDEVPOP" => AggregateFunction::StddevPop,
            "VARSAMP" => AggregateFunction::VarSamp,
            "STDDEVSAMP" => AggregateFunction::StddevSamp,
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
        Some(Token::Word(word)) if word.eq_ignore_ascii_case("NULL") || scalar::is_function(word)
    ) {
        let mut columns = Vec::new();
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
        if let [
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
        let mut grouped_aggregate = None;
        while parser.take(Token::Comma) {
            if columns.len() == 1
                && matches!(parser.tokens.get(parser.index), Some(Token::Word(word)) if word.eq_ignore_ascii_case("COUNT"))
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen)
            {
                parser.keyword("COUNT");
                if !parser.take(Token::LParen) {
                    return Err(ParseError("expected COUNT() or COUNT(*)".into()));
                }
                if !parser.take(Token::RParen)
                    && !(parser.take(Token::Star) && parser.take(Token::RParen))
                {
                    return Err(ParseError("expected COUNT() or COUNT(*)".into()));
                }
                grouped_count = true;
                break;
            }
            if columns.len() == 1 && parser.tokens.get(parser.index + 1) == Some(&Token::LParen) {
                let function = match parser.tokens.get(parser.index) {
                    Some(Token::Word(name)) => match name.to_ascii_uppercase().as_str() {
                        "SUM" => Some(AggregateFunction::Sum),
                        "AVG" => Some(AggregateFunction::Avg),
                        "MIN" => Some(AggregateFunction::Min),
                        "MAX" => Some(AggregateFunction::Max),
                        "RMS" => Some(AggregateFunction::Rms),
                        "VARPOP" => Some(AggregateFunction::VarPop),
                        "STDDEVPOP" => Some(AggregateFunction::StddevPop),
                        "VARSAMP" => Some(AggregateFunction::VarSamp),
                        "STDDEVSAMP" => Some(AggregateFunction::StddevSamp),
                        "MEDIAN" => Some(AggregateFunction::Median),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(function) = function {
                    parser.index += 1;
                    parser.take(Token::LParen);
                    let value_column = parser.word()?;
                    if !parser.take(Token::RParen) {
                        return Err(ParseError("expected ')' after grouped aggregate".into()));
                    }
                    grouped_aggregate = Some((function, value_column));
                    break;
                }
            }
            columns.push(parser.word()?);
        }
        if grouped_count {
            Projection::GroupedCount {
                column: columns.remove(0),
            }
        } else if let Some((function, value_column)) = grouped_aggregate {
            Projection::GroupedAggregate {
                column: columns.remove(0),
                function,
                value_column,
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
                | Projection::Aggregate { .. }
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
                if count > 1_000_000 {
                    return Err(ParseError("numbers supports at most 1000000 rows".into()));
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
                if first > 1_000_000 {
                    return Err(ParseError("numbers supports at most 1000000 rows".into()));
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
                    ParseError("generate_series supports at most 1000000 rows".into())
                })?
            };
            if count > 1_000_000 {
                return Err(ParseError(
                    "generate_series supports at most 1000000 rows".into(),
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

    let (predicates, or_predicates) = if parser.keyword("WHERE") {
        if source.is_none() {
            return Err(ParseError("WHERE requires a table source".into()));
        }
        let mut branches: Vec<Vec<Predicate>> = vec![Vec::new()];
        let mut total_conditions = 0usize;
        loop {
            if total_conditions == 16 {
                return Err(ParseError("WHERE supports at most 16 conditions".into()));
            }
            let prefix_not = parser.keyword("NOT");
            let column = parser.word()?;
            let postfix_not = parser.keyword("NOT");
            let negate_in = prefix_not ^ postfix_not;
            if parser.keyword("IS") {
                if postfix_not {
                    return Err(ParseError("unexpected NOT before IS".into()));
                }
                let negated = parser.keyword("NOT") ^ prefix_not;
                if !parser.keyword("NULL") {
                    return Err(ParseError("expected NULL after IS".into()));
                }
                branches
                    .last_mut()
                    .unwrap()
                    .push(Predicate::IsNull { column, negated });
                total_conditions += 1;
            } else if let Some(case_insensitive) = parser.like_keyword() {
                let pattern = match parser.tokens.get(parser.index) {
                    Some(Token::String(value)) => value.clone(),
                    _ => return Err(ParseError("LIKE requires a string literal".into())),
                };
                let trailing_escapes = pattern
                    .as_bytes()
                    .iter()
                    .rev()
                    .take_while(|&&byte| byte == b'\\')
                    .count();
                if trailing_escapes % 2 != 0 {
                    return Err(ParseError("LIKE pattern has a trailing escape".into()));
                }
                parser.index += 1;
                branches.last_mut().unwrap().push(Predicate::Like {
                    column,
                    pattern,
                    case_insensitive,
                    negated: negate_in,
                });
                total_conditions += 1;
            } else if parser.keyword("BETWEEN") {
                let lower = parser.number()?;
                if !parser.keyword("AND") {
                    return Err(ParseError("BETWEEN requires AND".into()));
                }
                let upper = parser.number()?;
                if negate_in {
                    branches.last_mut().unwrap().push(Predicate::NotBetween {
                        column,
                        lower,
                        upper,
                    });
                    total_conditions += 1;
                } else {
                    if total_conditions + 2 > 16 {
                        return Err(ParseError("WHERE supports at most 16 conditions".into()));
                    }
                    branches.last_mut().unwrap().push(Predicate::Comparison {
                        column: column.clone(),
                        operator: Operator::Ge,
                        value: lower,
                    });
                    branches.last_mut().unwrap().push(Predicate::Comparison {
                        column,
                        operator: Operator::Le,
                        value: upper,
                    });
                    total_conditions += 2;
                }
            } else if parser.keyword("IN") {
                if !parser.take(Token::LParen) {
                    return Err(ParseError("expected '(' after IN".into()));
                }
                let mut values = Vec::new();
                loop {
                    if values.len() == 32 {
                        return Err(ParseError("IN supports at most 32 values".into()));
                    }
                    values.push(parser.literal()?);
                    if !parser.take(Token::Comma) {
                        break;
                    }
                }
                if !parser.take(Token::RParen) {
                    return Err(ParseError("expected ')' after IN list".into()));
                }
                branches.last_mut().unwrap().push(if negate_in {
                    Predicate::NotIn { column, values }
                } else {
                    Predicate::In { column, values }
                });
                total_conditions += 1;
            } else {
                if postfix_not {
                    return Err(ParseError(
                        "NOT after a column requires IN or BETWEEN".into(),
                    ));
                }
                let mut operator = if parser.take(Token::Percent) {
                    let modulus = parser.number()?;
                    let modulus = modulus
                        .parse::<u64>()
                        .map_err(|_| ParseError("modulus must be a positive UInt64".into()))?;
                    if modulus == 0 {
                        return Err(ParseError("modulus must be positive".into()));
                    }
                    match parser.operator()? {
                        Operator::Eq => Operator::Modulo(modulus),
                        Operator::Ne => Operator::ModuloNe(modulus),
                        _ => return Err(ParseError("modulo predicates require = or !=".into())),
                    }
                } else {
                    parser.operator()?
                };
                if prefix_not {
                    operator = match operator {
                        Operator::Eq => Operator::Ne,
                        Operator::Ne => Operator::Eq,
                        Operator::Lt => Operator::Ge,
                        Operator::Le => Operator::Gt,
                        Operator::Gt => Operator::Le,
                        Operator::Ge => Operator::Lt,
                        Operator::Modulo(_) | Operator::ModuloNe(_) => {
                            return Err(ParseError("NOT modulo predicates are unsupported".into()));
                        }
                    };
                }
                branches.last_mut().unwrap().push(Predicate::Comparison {
                    column,
                    operator,
                    value: parser.literal()?,
                });
                total_conditions += 1;
            }
            if parser.keyword("AND") {
                continue;
            }
            if parser.keyword("OR") {
                branches.push(Vec::new());
                continue;
            }
            break;
        }
        let first = branches.remove(0);
        (first, branches)
    } else {
        (Vec::new(), Vec::new())
    };

    if parser.keyword("GROUP") {
        if !parser.keyword("BY") {
            return Err(ParseError("expected BY after GROUP".into()));
        }
        let group_column = parser.word()?;
        match &projection {
            Projection::GroupedCount { column } | Projection::GroupedAggregate { column, .. }
                if column == &group_column => {}
            Projection::GroupedCount { .. } | Projection::GroupedAggregate { .. } => {
                return Err(ParseError("GROUP BY must match the selected key".into()));
            }
            _ => {
                return Err(ParseError(
                    "GROUP BY requires SELECT key, aggregate(column)".into(),
                ));
            }
        }
    } else if matches!(
        projection,
        Projection::GroupedCount { .. } | Projection::GroupedAggregate { .. }
    ) {
        return Err(ParseError("grouped aggregates require GROUP BY".into()));
    }

    let having = if parser.keyword("HAVING") {
        if !matches!(projection, Projection::GroupedCount { .. }) {
            return Err(ParseError("HAVING requires GROUP BY with COUNT(*)".into()));
        }
        if !parser.keyword("COUNT") || !parser.take(Token::LParen) {
            return Err(ParseError("HAVING requires COUNT(*)".into()));
        }
        if !parser.take(Token::RParen) && !(parser.take(Token::Star) && parser.take(Token::RParen))
        {
            return Err(ParseError("HAVING requires COUNT() or COUNT(*)".into()));
        }
        let operator = parser.operator()?;
        let value = parser
            .number()?
            .parse::<u64>()
            .map_err(|_| ParseError("HAVING count must be a UInt64".into()))?;
        Some(Having { operator, value })
    } else {
        None
    };

    let mut order_by_extra = Vec::new();
    let order_by = if parser.keyword("ORDER") {
        if !parser.keyword("BY") {
            return Err(ParseError("expected BY after ORDER".into()));
        }
        let first = parser.order_term(&projection)?;
        while parser.take(Token::Comma) {
            if order_by_extra.len() >= 7 {
                return Err(ParseError("ORDER BY supports at most 8 columns".into()));
            }
            order_by_extra.push(parser.order_term(&projection)?);
        }
        Some(first)
    } else {
        None
    };
    if order_by.is_some()
        && matches!(
            projection,
            Projection::Count
                | Projection::CountColumn { .. }
                | Projection::CountDistinct { .. }
                | Projection::Aggregate { .. }
                | Projection::Literal(_)
        )
    {
        return Err(ParseError("ORDER BY requires column projections".into()));
    }

    if let Projection::GroupedCount { column } = &projection {
        if order_by
            .iter()
            .chain(order_by_extra.iter())
            .any(|order| &order.column != column && order.column != "count")
        {
            return Err(ParseError("grouped ORDER BY must use the group key".into()));
        }
    }
    if let Projection::GroupedAggregate { column, .. } = &projection {
        if order_by
            .as_ref()
            .is_some_and(|order| &order.column != column)
        {
            return Err(ParseError(
                "grouped numeric ORDER BY must use the group key".into(),
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
    if offset.is_some() && limit.is_none() {
        return Err(ParseError("OFFSET requires LIMIT or FETCH".into()));
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
