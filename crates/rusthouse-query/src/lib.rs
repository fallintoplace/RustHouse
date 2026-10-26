mod ddl;
mod execute;
mod lexer;
pub use ddl::{ColumnDef, Ddl, parse_ddl};
pub use execute::{ExecutionError, execute};

use std::error::Error;
use std::fmt;

use lexer::{Token, tokenize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateFunction {
    Sum,
    Avg,
    Min,
    Max,
    VarPop,
    StddevPop,
}

impl AggregateFunction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
            Self::VarPop => "varPop",
            Self::StddevPop => "stddevPop",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Projection {
    Literal(i64),
    Count,
    CountDistinct {
        column: String,
    },
    GroupedCount {
        column: String,
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
}

impl Predicate {
    fn column(&self) -> &str {
        match self {
            Self::Comparison { column, .. }
            | Self::In { column, .. }
            | Self::NotIn { column, .. }
            | Self::NotBetween { column, .. } => column,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Table(String),
    Numbers(u64),
    NumbersRange { start: u64, count: u64 },
    NumbersStepped { start: u64, length: u64, step: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderBy {
    pub column: String,
    pub descending: bool,
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
    pub distinct: bool,
    pub source: Option<Source>,
    pub predicates: Vec<Predicate>,
    /// Additional OR branches, each containing AND-connected predicates.
    pub or_predicates: Vec<Vec<Predicate>>,
    pub order_by: Option<OrderBy>,
    pub having: Option<Having>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
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

    fn word(&mut self) -> Result<String, ParseError> {
        match self.tokens.get(self.index) {
            Some(Token::Word(value)) => {
                self.index += 1;
                Ok(value.clone())
            }
            _ => Err(ParseError("expected an identifier".into())),
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
}

pub fn parse_select(sql: &str) -> Result<Select, ParseError> {
    let mut parser = Parser {
        tokens: tokenize(sql)?,
        index: 0,
    };
    if !parser.keyword("SELECT") {
        return Err(ParseError("only SELECT statements are supported".into()));
    }
    let distinct = parser.keyword("DISTINCT");

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
        } else {
            if !parser.take(Token::RParen)
                && !(parser.take(Token::Star) && parser.take(Token::RParen))
            {
                return Err(ParseError("expected COUNT() or COUNT(*)".into()));
            }
            Projection::Count
        }
    } else if matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen)) {
        let name = parser.word()?;
        let function = match name.to_ascii_uppercase().as_str() {
            "SUM" => AggregateFunction::Sum,
            "AVG" => AggregateFunction::Avg,
            "MIN" => AggregateFunction::Min,
            "MAX" => AggregateFunction::Max,
            "VARPOP" => AggregateFunction::VarPop,
            "STDDEVPOP" => AggregateFunction::StddevPop,
            _ => return Err(ParseError(format!("unsupported aggregate: {name}"))),
        };
        parser.take(Token::LParen);
        let column = parser.word()?;
        if !parser.take(Token::RParen) {
            return Err(ParseError("expected ')' after aggregate column".into()));
        }
        Projection::Aggregate { function, column }
    } else if matches!(parser.tokens.get(parser.index), Some(Token::Number(_))) {
        let literal = parser
            .number()?
            .parse::<i64>()
            .map_err(|_| ParseError("SELECT literals must be signed 64-bit integers".into()))?;
        Projection::Literal(literal)
    } else {
        let mut columns = vec![parser.word()?];
        let mut grouped_count = false;
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
            columns.push(parser.word()?);
        }
        if grouped_count {
            Projection::GroupedCount {
                column: columns.remove(0),
            }
        } else {
            Projection::Columns(columns)
        }
    };

    if distinct && !matches!(projection, Projection::All | Projection::Columns(_)) {
        return Err(ParseError("DISTINCT supports only row projections".into()));
    }
    let alias = if parser.keyword("AS") {
        let single_result = matches!(
            &projection,
            Projection::Literal(_)
                | Projection::Count
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
                if first.checked_add(count).is_none() {
                    return Err(ParseError("numbers range overflows UInt64".into()));
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
            if !parser.take(Token::RParen) {
                return Err(ParseError("expected ')' after numbers row count".into()));
            }
            Some(source)
        } else {
            Some(Source::Table(name))
        }
    } else {
        None
    };
    if source.is_none() && !matches!(projection, Projection::Literal(_)) {
        return Err(ParseError("a table source is required".into()));
    }
    if source.is_some() && matches!(projection, Projection::Literal(_)) {
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
            if parser.keyword("BETWEEN") {
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
                    values.push(parser.number()?);
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
                    value: parser.number()?,
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
            Projection::GroupedCount { column } if column == &group_column => {}
            Projection::GroupedCount { .. } => {
                return Err(ParseError("GROUP BY must match the selected key".into()));
            }
            _ => {
                return Err(ParseError("GROUP BY requires SELECT key, COUNT(*)".into()));
            }
        }
    } else if matches!(projection, Projection::GroupedCount { .. }) {
        return Err(ParseError("COUNT(*) with a key requires GROUP BY".into()));
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

    let order_by = if parser.keyword("ORDER") {
        if !parser.keyword("BY") {
            return Err(ParseError("expected BY after ORDER".into()));
        }
        let column = parser.word()?;
        if matches!(projection, Projection::GroupedCount { .. })
            && column.eq_ignore_ascii_case("count")
            && parser.take(Token::LParen)
            && !parser.take(Token::RParen)
            && !(parser.take(Token::Star) && parser.take(Token::RParen))
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
        let descending = if parser.keyword("DESC") {
            true
        } else {
            parser.keyword("ASC");
            false
        };
        Some(OrderBy { column, descending })
    } else {
        None
    };
    if order_by.is_some()
        && matches!(
            projection,
            Projection::Count
                | Projection::CountDistinct { .. }
                | Projection::Aggregate { .. }
                | Projection::Literal(_)
        )
    {
        return Err(ParseError("ORDER BY requires column projections".into()));
    }

    if let Projection::GroupedCount { column } = &projection {
        if order_by
            .as_ref()
            .is_some_and(|order| &order.column != column && order.column != "count")
        {
            return Err(ParseError("grouped ORDER BY must use the group key".into()));
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
            return Err(ParseError("FETCH count must be a non-negative integer".into()));
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
        distinct,
        source,
        predicates,
        or_predicates,
        order_by,
        having,
        limit,
        offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scalar_and_table_queries() {
        assert_eq!(
            parse_select(" SELECT -42; ").unwrap(),
            Select {
                projection: Projection::Literal(-42),
                alias: None,
                distinct: false,
                source: None,
                predicates: Vec::new(),
                or_predicates: Vec::new(),
                order_by: None,
                having: None,
                limit: None,
                offset: None,
            }
        );
        assert_eq!(
            parse_select("select id, price from part limit 2;").unwrap(),
            Select {
                projection: Projection::Columns(vec!["id".into(), "price".into()]),
                alias: None,
                distinct: false,
                source: Some(Source::Table("part".into())),
                predicates: Vec::new(),
                or_predicates: Vec::new(),
                order_by: None,
                having: None,
                limit: Some(2),
                offset: None,
            }
        );
        assert_eq!(
            parse_select("SELECT count, id FROM part")
                .unwrap()
                .projection,
            Projection::Columns(vec!["count".into(), "id".into()])
        );
        assert_eq!(
            parse_select("SELECT * FROM part").unwrap().projection,
            Projection::All
        );
        assert_eq!(
            parse_select("SeLeCt count(*) FROM part")
                .unwrap()
                .projection,
            Projection::Count
        );
    }

    #[test]
    fn parses_single_column_result_aliases() {
        assert_eq!(
            parse_select("SELECT number AS n FROM numbers(10)")
                .unwrap()
                .alias
                .as_deref(),
            Some("n")
        );
        assert_eq!(
            parse_select("SELECT count(*) AS total FROM numbers(10)")
                .unwrap()
                .alias
                .as_deref(),
            Some("total")
        );
        assert_eq!(
            parse_select("SELECT -3 AS result")
                .unwrap()
                .alias
                .as_deref(),
            Some("result")
        );
        for sql in [
            "SELECT number AS FROM numbers(5)",
            "SELECT number, number AS alias FROM numbers(5)",
            "SELECT * AS everything FROM numbers(5)",
            "SELECT number AS 3 FROM numbers(5)",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_generated_numbers_and_rejects_invalid_counts() {
        let query =
            parse_select("SELECT number FROM numbers(12) WHERE number >= 5 LIMIT 2").unwrap();
        assert_eq!(query.source, Some(Source::Numbers(12)));
        assert_eq!(query.projection, Projection::Columns(vec!["number".into()]));
        assert_eq!(
            parse_select("SELECT count(*) FROM NUMBERS(0)")
                .unwrap()
                .source,
            Some(Source::Numbers(0))
        );
        for sql in [
            "SELECT number FROM numbers()",
            "SELECT number FROM numbers(-1)",
            "SELECT number FROM numbers(1.5)",
            "SELECT number FROM numbers(1000001)",
            "SELECT number FROM numbers(18446744073709551616)",
            "SELECT number FROM numbers(3",
            "SELECT number FROM numbers",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_numbers_start_count_with_checked_bounds() {
        assert_eq!(
            parse_select("SELECT number FROM numbers(10, 3)")
                .unwrap()
                .source,
            Some(Source::NumbersRange {
                start: 10,
                count: 3
            })
        );
        assert_eq!(
            parse_select("SELECT count(*) FROM numbers(18446744073709551615, 0)")
                .unwrap()
                .source,
            Some(Source::NumbersRange {
                start: u64::MAX,
                count: 0
            })
        );
        for sql in [
            "SELECT number FROM numbers(1,)",
            "SELECT number FROM numbers(1,-1)",
            "SELECT number FROM numbers(1,1000001)",
            "SELECT number FROM numbers(18446744073709551615,2)",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_stepped_numbers_with_bounded_ranges() {
        let query = parse_select("SELECT number FROM numbers(4, 9, 3)").unwrap();
        assert_eq!(
            query.source,
            Some(Source::NumbersStepped {
                start: 4,
                length: 9,
                step: 3,
            })
        );
        for sql in [
            "SELECT number FROM numbers(0, 10, 0)",
            "SELECT number FROM numbers(0, 10, -1)",
            "SELECT number FROM numbers(0, 10, 2, 3)",
            "SELECT number FROM numbers(18446744073709551615, 2, 1)",
            "SELECT number FROM numbers(0, 1000001, 2)",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_numeric_aggregates_and_rejects_unsupported_functions() {
        for (name, expected) in [
            ("SUM", AggregateFunction::Sum),
            ("AVG", AggregateFunction::Avg),
            ("MIN", AggregateFunction::Min),
            ("MAX", AggregateFunction::Max),
            ("VARPOP", AggregateFunction::VarPop),
            ("STDDEVPOP", AggregateFunction::StddevPop),
        ] {
            let sql = format!("SELECT {name}(price) FROM part WHERE id > 2");
            assert_eq!(
                parse_select(&sql).unwrap().projection,
                Projection::Aggregate {
                    function: expected,
                    column: "price".into(),
                }
            );
        }
        for sql in [
            "SELECT sum(*) FROM part",
            "SELECT sum() FROM part",
            "SELECT sum(a, b) FROM part",
            "SELECT median(price) FROM part",
            "SELECT avg(number) FROM numbers(3) ORDER BY number",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_count_distinct_projection() {
        assert_eq!(
            parse_select("SELECT count(DISTINCT number) FROM numbers(10)")
                .unwrap()
                .projection,
            Projection::CountDistinct {
                column: "number".into(),
            }
        );
        for sql in [
            "SELECT count(DISTINCT) FROM numbers(10)",
            "SELECT count(DISTINCT number, other) FROM numbers(10)",
            "SELECT DISTINCT count(DISTINCT number) FROM numbers(10)",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_order_by_and_rejects_invalid_ordering() {
        let select = parse_select(
            "SELECT number FROM numbers(10) WHERE number > 2 ORDER BY number DESC LIMIT 3",
        )
        .unwrap();
        assert_eq!(
            select.order_by,
            Some(OrderBy {
                column: "number".into(),
                descending: true,
            })
        );
        assert_eq!(
            parse_select("SELECT id FROM part ORDER BY price ASC")
                .unwrap()
                .order_by,
            Some(OrderBy {
                column: "price".into(),
                descending: false,
            })
        );
        for sql in [
            "SELECT number FROM numbers(3) ORDER number",
            "SELECT number FROM numbers(3) ORDER BY",
            "SELECT number FROM numbers(3) ORDER BY number DESC ASC",
            "SELECT number FROM numbers(3) LIMIT 1 ORDER BY number",
            "SELECT count(*) FROM numbers(3) ORDER BY number",
            "SELECT 1 ORDER BY number",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn grouped_queries_can_order_by_count() {
        for order in ["count", "count()", "count(*)"] {
            let sql = format!(
                "SELECT number, count(*) FROM numbers(10) GROUP BY number ORDER BY {order} DESC"
            );
            let query = parse_select(&sql).unwrap();
            assert_eq!(
                query.order_by,
                Some(OrderBy {
                    column: "count".into(),
                    descending: true,
                })
            );
        }
        assert!(
            parse_select(
                "SELECT number, count(*) FROM numbers(10) GROUP BY number ORDER BY count(foo)"
            )
            .is_err()
        );
    }

    #[test]
    fn parses_numeric_where_operators() {
        for (input, expected) in [
            ("=", Operator::Eq),
            ("!=", Operator::Ne),
            ("<>", Operator::Ne),
            ("<", Operator::Lt),
            ("<=", Operator::Le),
            (">", Operator::Gt),
            (">=", Operator::Ge),
        ] {
            let sql = format!("SELECT id FROM part WHERE id {input} -12.5 LIMIT 0");
            let query = parse_select(&sql).unwrap();
            assert_eq!(
                query.predicates.into_iter().next().unwrap(),
                Predicate::Comparison {
                    column: "id".into(),
                    operator: expected,
                    value: "-12.5".into(),
                }
            );
            assert_eq!(query.limit, Some(0));
        }
    }

    #[test]
    fn rejects_malformed_and_unsupported_statements() {
        for sql in [
            "",
            "DELETE FROM part",
            "SELECT *",
            "SELECT 1 FROM part",
            "SELECT 1; SELECT 2",
            "SELECT 1;;",
            "SELECT id, FROM part",
            "SELECT id FROM part LIMIT -1",
            "SELECT id FROM part LIMIT 1.5",
            "SELECT id FROM part WHERE id > 'abc'",
            "SELECT id FROM part WHERE id =",
            "SELECT id FROM part LIMIT 184467440737095516161",
            "SELECT 1 + 2",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_and_conditions_and_rejects_incomplete_filters() {
        let query =
            parse_select("SELECT number FROM numbers(12) WHERE number >= 3 AND number < 8 LIMIT 4")
                .unwrap();
        assert_eq!(
            query.predicates,
            vec![
                Predicate::Comparison {
                    column: "number".into(),
                    operator: Operator::Ge,
                    value: "3".into(),
                },
                Predicate::Comparison {
                    column: "number".into(),
                    operator: Operator::Lt,
                    value: "8".into(),
                },
            ]
        );
        for sql in [
            "SELECT number FROM numbers(5) WHERE number > 1 AND",
            "SELECT number FROM numbers(5) WHERE number > 1 AND AND number < 4",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
        let many = format!(
            "SELECT number FROM numbers(5) WHERE {}",
            vec!["number > 0"; 17].join(" AND ")
        );
        assert!(parse_select(&many).is_err());
    }

    #[test]
    fn parses_between_as_inclusive_range_predicates() {
        let query = parse_select(
            "SELECT number FROM numbers(12) WHERE number BETWEEN 3 AND 8 AND number != 4 LIMIT 3",
        )
        .unwrap();
        assert_eq!(
            query.predicates,
            vec![
                Predicate::Comparison {
                    column: "number".into(),
                    operator: Operator::Ge,
                    value: "3".into(),
                },
                Predicate::Comparison {
                    column: "number".into(),
                    operator: Operator::Le,
                    value: "8".into(),
                },
                Predicate::Comparison {
                    column: "number".into(),
                    operator: Operator::Ne,
                    value: "4".into(),
                },
            ]
        );
        for sql in [
            "SELECT number FROM numbers(5) WHERE number BETWEEN",
            "SELECT number FROM numbers(5) WHERE number BETWEEN 1",
            "SELECT number FROM numbers(5) WHERE number BETWEEN 1 OR 2",
            "SELECT number FROM numbers(5) WHERE number BETWEEN 1 AND",
            "SELECT number FROM numbers(5) WHERE number BETWEEN 1 AND 3 AND",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
        let too_many = format!(
            "SELECT number FROM numbers(5) WHERE {} AND number BETWEEN 1 AND 2",
            vec!["number > 0"; 15].join(" AND ")
        );
        assert!(parse_select(&too_many).is_err());
    }

    #[test]
    fn parses_not_between_with_other_predicates() {
        let parsed = parse_select(
            "SELECT number FROM numbers(10) WHERE number NOT BETWEEN 3 AND 6 AND number < 9",
        )
        .unwrap();
        assert_eq!(
            parsed.predicates[0],
            Predicate::NotBetween {
                column: "number".into(),
                lower: "3".into(),
                upper: "6".into(),
            }
        );
        assert!(parse_select("SELECT number FROM numbers(5) WHERE number NOT BETWEEN 2").is_err());
    }

    #[test]
    fn parses_grouped_counts_and_rejects_unsupported_grouping() {
        let query = parse_select(
            "SELECT id, COUNT(*) FROM part WHERE id IN (2, 4) GROUP BY id ORDER BY id DESC LIMIT 2 OFFSET 1",
        )
        .unwrap();
        assert_eq!(
            query.projection,
            Projection::GroupedCount {
                column: "id".into(),
            }
        );
        assert_eq!(
            query.order_by,
            Some(OrderBy {
                column: "id".into(),
                descending: true,
            })
        );
        for sql in [
            "SELECT id, count(*) FROM part",
            "SELECT id, count(*) FROM part GROUP BY price",
            "SELECT id, count(*) FROM part GROUP id",
            "SELECT id, count(*) FROM part GROUP BY id, price",
            "SELECT id, count(*) FROM part GROUP BY id ORDER BY price",
            "SELECT id FROM part GROUP BY id",
            "SELECT count(*) FROM part GROUP BY id",
            "SELECT id, sum(price) FROM part GROUP BY id",
            "SELECT DISTINCT id, count(*) FROM part GROUP BY id",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_distinct_and_rejects_invalid_projections() {
        let query = parse_select(
            "SELECT DISTINCT id, price FROM part WHERE id > 1 ORDER BY price DESC LIMIT 3",
        )
        .unwrap();
        assert!(query.distinct);
        assert_eq!(
            query.projection,
            Projection::Columns(vec!["id".into(), "price".into()])
        );
        assert!(
            parse_select("SELECT DISTINCT * FROM part")
                .unwrap()
                .distinct
        );
        for sql in [
            "SELECT DISTINCT 1",
            "SELECT DISTINCT count(*) FROM part",
            "SELECT DISTINCT sum(id) FROM part",
            "SELECT DISTINCT DISTINCT id FROM part",
            "SELECT DISTINCT FROM part",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_having_counts_after_grouping() {
        let query = parse_select(
            "SELECT number, count(*) FROM numbers(10) GROUP BY number HAVING count(*) >= 2 ORDER BY number",
        )
        .unwrap();
        assert_eq!(
            query.having,
            Some(Having {
                operator: Operator::Ge,
                value: 2,
            })
        );
        for sql in [
            "SELECT number FROM numbers(10) HAVING count(*) > 1",
            "SELECT number, count(*) FROM numbers(10) GROUP BY number HAVING sum(number) > 1",
            "SELECT number, count(*) FROM numbers(10) GROUP BY number HAVING count(*) >= -1",
            "SELECT number, count(*) FROM numbers(10) GROUP BY number HAVING count(*) >",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_or_with_and_precedence() {
        let query = parse_select(
            "SELECT number FROM numbers(10) WHERE number = 1 OR number >= 4 AND number < 6",
        )
        .unwrap();
        assert_eq!(query.predicates.len(), 1);
        assert_eq!(query.or_predicates.len(), 1);
        assert_eq!(query.or_predicates[0].len(), 2);
        for sql in [
            "SELECT number FROM numbers(10) WHERE number = 1 OR",
            "SELECT number FROM numbers(10) WHERE number = 1 OR OR number = 2",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_in_and_checks_list_bounds() {
        let query =
            parse_select("SELECT number FROM numbers(9) WHERE number IN (1, 4, 7) AND number > 2")
                .unwrap();
        assert_eq!(
            query.predicates,
            vec![
                Predicate::In {
                    column: "number".into(),
                    values: vec!["1".into(), "4".into(), "7".into()],
                },
                Predicate::Comparison {
                    column: "number".into(),
                    operator: Operator::Gt,
                    value: "2".into(),
                },
            ]
        );
        for sql in [
            "SELECT number FROM numbers(5) WHERE number IN ()",
            "SELECT number FROM numbers(5) WHERE number IN (1,)",
            "SELECT number FROM numbers(5) WHERE number IN (1 2)",
            "SELECT number FROM numbers(5) WHERE number IN 1",
            "SELECT number FROM numbers(5) WHERE number IN (1, 2",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
        let too_many = format!(
            "SELECT number FROM numbers(9) WHERE number IN ({})",
            vec!["1"; 33].join(",")
        );
        assert!(parse_select(&too_many).is_err());
    }

    #[test]
    fn parses_modulo_filters_and_rejects_invalid_divisors() {
        let query = parse_select("SELECT number FROM numbers(10) WHERE number % 3 = 1").unwrap();
        assert!(matches!(
            query.predicates[0],
            Predicate::Comparison {
                operator: Operator::Modulo(3),
                ..
            }
        ));
        assert!(matches!(
            parse_select("SELECT number FROM numbers(10) WHERE number % 3 != 1")
                .unwrap()
                .predicates[0],
            Predicate::Comparison {
                operator: Operator::ModuloNe(3),
                ..
            }
        ));
        for sql in [
            "SELECT number FROM numbers(10) WHERE number % 0 = 1",
            "SELECT number FROM numbers(10) WHERE number % -1 = 0",
            "SELECT number FROM numbers(10) WHERE number % 2 > 0",
            "SELECT number FROM numbers(10) WHERE number % 2.5 = 1",
            "SELECT number FROM numbers(10) WHERE number % 2 =",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_not_in_numeric_lists() {
        let query =
            parse_select("SELECT number FROM numbers(10) WHERE number NOT IN (1, 4, 7)").unwrap();
        assert_eq!(
            query.predicates,
            vec![Predicate::NotIn {
                column: "number".into(),
                values: vec!["1".into(), "4".into(), "7".into()],
            }]
        );
        for sql in [
            "SELECT number FROM numbers(10) WHERE number NOT IN ()",
            "SELECT number FROM numbers(10) WHERE number NOT IN (1,)",
            "SELECT number FROM numbers(10) WHERE number NOT = 1",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_limit_offset_and_rejects_invalid_pagination() {
        let query =
            parse_select("SELECT number FROM numbers(10) ORDER BY number DESC LIMIT 3 OFFSET 2;")
                .unwrap();
        assert_eq!(query.limit, Some(3));
        assert_eq!(query.offset, Some(2));
        assert_eq!(
            parse_select("SELECT id FROM part LIMIT 0 OFFSET 0")
                .unwrap()
                .offset,
            Some(0)
        );
        for sql in [
            "SELECT number FROM numbers(10) OFFSET 2",
            "SELECT number FROM numbers(10) LIMIT 2 OFFSET -1",
            "SELECT number FROM numbers(10) LIMIT 2 OFFSET 1.5",
            "SELECT number FROM numbers(10) LIMIT 2 OFFSET",
            "SELECT number FROM numbers(10) LIMIT 2 OFFSET 184467440737095516160",
            "SELECT number FROM numbers(10) LIMIT 2 OFFSET 1 OFFSET 2",
            "SELECT number FROM numbers(10) LIMIT 2 OFFSET 1 ORDER BY number",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn supports_clickhouse_limit_offset_count_syntax() {
        let query = parse_select("SELECT number FROM numbers(20) LIMIT 3, 2").unwrap();
        assert_eq!(query.limit, Some(2));
        assert_eq!(query.offset, Some(3));
        for sql in [
            "SELECT number FROM numbers(20) LIMIT 1,",
            "SELECT number FROM numbers(20) LIMIT 1, -2",
            "SELECT number FROM numbers(20) LIMIT 1, 2 OFFSET 3",
            "SELECT number FROM numbers(20) LIMIT 1, 184467440737095516160",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn supports_clickhouse_count_without_arguments() {
        assert_eq!(
            parse_select("SELECT count() FROM numbers(10)")
                .unwrap()
                .projection,
            Projection::Count
        );
        assert_eq!(
            parse_select("SELECT number, count() FROM numbers(10) GROUP BY number")
                .unwrap()
                .projection,
            Projection::GroupedCount {
                column: "number".into(),
            }
        );
        for sql in [
            "SELECT count( FROM numbers(2)",
            "SELECT count(,) FROM numbers(2)",
            "SELECT number, count( FROM numbers(2) GROUP BY number",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn bounds_sql_input_and_rejects_invalid_characters() {
        assert!(parse_select(&" ".repeat(4097)).is_err());
        assert!(parse_select("SELECT café FROM part").is_err());
    }
    #[test]
    fn parses_unary_not_on_numeric_predicates() {
        let query = parse_select("SELECT number FROM numbers(6) WHERE NOT number < 3").unwrap();
        assert_eq!(
            query.predicates,
            vec![Predicate::Comparison {
                column: "number".into(),
                operator: Operator::Ge,
                value: "3".into(),
            }]
        );
        assert!(matches!(
            parse_select("SELECT number FROM numbers(6) WHERE NOT number IN (1, 3)")
                .unwrap()
                .predicates[0],
            Predicate::NotIn { .. }
        ));
        assert!(matches!(
            parse_select("SELECT number FROM numbers(6) WHERE NOT number BETWEEN 2 AND 4")
                .unwrap()
                .predicates[0],
            Predicate::NotBetween { .. }
        ));
        let twice =
            parse_select("SELECT number FROM numbers(6) WHERE NOT number NOT BETWEEN 2 AND 4")
                .unwrap();
        assert_eq!(twice.predicates.len(), 2);
        for sql in [
            "SELECT number FROM numbers(6) WHERE number NOT = 3",
            "SELECT number FROM numbers(6) WHERE NOT number % 2 = 1",
            "SELECT number FROM numbers(6) WHERE NOT",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn parses_standard_fetch_pagination() {
        let query = parse_select(
            "SELECT number FROM numbers(10) ORDER BY number DESC OFFSET 2 ROWS FETCH NEXT 3 ROWS ONLY",
        )
        .unwrap();
        assert_eq!(query.limit, Some(3));
        assert_eq!(query.offset, Some(2));
        let fetch =
            parse_select("SELECT number FROM numbers(10) FETCH FIRST 0 ROW ONLY").unwrap();
        assert_eq!(fetch.limit, Some(0));
        assert_eq!(fetch.offset, None);
        let legacy = parse_select("SELECT number FROM numbers(10) LIMIT 3 OFFSET 2").unwrap();
        assert_eq!(legacy.limit, query.limit);
        assert_eq!(legacy.offset, query.offset);
    }

    #[test]
    fn rejects_malformed_fetch_clauses() {
        for sql in [
            "SELECT number FROM numbers(10) FETCH 3 ROWS ONLY",
            "SELECT number FROM numbers(10) FETCH FIRST -1 ROWS ONLY",
            "SELECT number FROM numbers(10) FETCH FIRST 3",
            "SELECT number FROM numbers(10) FETCH FIRST 3 ROWS",
            "SELECT number FROM numbers(10) LIMIT 2 FETCH FIRST 3 ROWS ONLY",
            "SELECT number FROM numbers(10) OFFSET 2 ROWS",
            "SELECT number FROM numbers(10) OFFSET -1 ROWS FETCH NEXT 2 ROWS ONLY",
            "SELECT number FROM numbers(10) FETCH NEXT 184467440737095516160 ROWS ONLY",
        ] {
            assert!(parse_select(sql).is_err(), "{sql}");
        }
    }

}
