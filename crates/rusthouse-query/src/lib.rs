mod execute;
pub use execute::{ExecutionError, execute};

use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateFunction {
    Sum,
    Avg,
    Min,
    Max,
}

impl AggregateFunction {
    pub fn name(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Projection {
    Literal(i64),
    Count,
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
}

impl Predicate {
    fn column(&self) -> &str {
        match self {
            Self::Comparison { column, .. } | Self::In { column, .. } => column,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Table(String),
    Numbers(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderBy {
    pub column: String,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Select {
    pub projection: Projection,
    pub distinct: bool,
    pub source: Option<Source>,
    pub predicates: Vec<Predicate>,
    /// Additional OR branches, each containing AND-connected predicates.
    pub or_predicates: Vec<Vec<Predicate>>,
    pub order_by: Option<OrderBy>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    Number(String),
    Star,
    Comma,
    LParen,
    RParen,
    Semicolon,
    Operator(Operator),
}

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
        if !parser.take(Token::LParen) || !parser.take(Token::Star) || !parser.take(Token::RParen) {
            return Err(ParseError("expected COUNT(*)".into()));
        }
        Projection::Count
    } else if matches!(parser.tokens.get(parser.index + 1), Some(Token::LParen)) {
        let name = parser.word()?;
        let function = match name.to_ascii_uppercase().as_str() {
            "SUM" => AggregateFunction::Sum,
            "AVG" => AggregateFunction::Avg,
            "MIN" => AggregateFunction::Min,
            "MAX" => AggregateFunction::Max,
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
        while parser.take(Token::Comma) {
            columns.push(parser.word()?);
        }
        Projection::Columns(columns)
    };

    if distinct && !matches!(projection, Projection::All | Projection::Columns(_)) {
        return Err(ParseError("DISTINCT supports only row projections".into()));
    }

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
            let count = count
                .parse::<u64>()
                .map_err(|_| ParseError("numbers row count is too large".into()))?;
            if count > 1_000_000 {
                return Err(ParseError("numbers supports at most 1000000 rows".into()));
            }
            if !parser.take(Token::RParen) {
                return Err(ParseError("expected ')' after numbers row count".into()));
            }
            Some(Source::Numbers(count))
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
            let column = parser.word()?;
            if parser.keyword("BETWEEN") {
                if total_conditions + 2 > 16 {
                    return Err(ParseError("WHERE supports at most 16 conditions".into()));
                }
                let lower = parser.number()?;
                if !parser.keyword("AND") {
                    return Err(ParseError("BETWEEN requires AND".into()));
                }
                let upper = parser.number()?;
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
                branches
                    .last_mut()
                    .unwrap()
                    .push(Predicate::In { column, values });
                total_conditions += 1;
            } else {
                branches.last_mut().unwrap().push(Predicate::Comparison {
                    column,
                    operator: parser.operator()?,
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

    let order_by = if parser.keyword("ORDER") {
        if !parser.keyword("BY") {
            return Err(ParseError("expected BY after ORDER".into()));
        }
        let column = parser.word()?;
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
            Projection::Count | Projection::Aggregate { .. } | Projection::Literal(_)
        )
    {
        return Err(ParseError("ORDER BY requires column projections".into()));
    }

    let limit = if parser.keyword("LIMIT") {
        let value = parser.number()?;
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError("LIMIT must be a non-negative integer".into()));
        }
        Some(
            value
                .parse::<usize>()
                .map_err(|_| ParseError("LIMIT is too large".into()))?,
        )
    } else {
        None
    };

    let offset = if parser.keyword("OFFSET") {
        if limit.is_none() {
            return Err(ParseError("OFFSET requires LIMIT".into()));
        }
        let value = parser.number()?;
        if !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError("OFFSET must be a non-negative integer".into()));
        }
        Some(
            value
                .parse::<usize>()
                .map_err(|_| ParseError("OFFSET is too large".into()))?,
        )
    } else {
        None
    };

    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError(
            "unexpected tokens after SELECT statement".into(),
        ));
    }
    Ok(Select {
        projection,
        distinct,
        source,
        predicates,
        or_predicates,
        order_by,
        limit,
        offset,
    })
}

fn tokenize(sql: &str) -> Result<Vec<Token>, ParseError> {
    if sql.len() > 4096 {
        return Err(ParseError("statement exceeds the 4096-byte limit".into()));
    }
    let bytes = sql.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let token = match byte {
            b' ' | b'\t' | b'\r' | b'\n' => {
                index += 1;
                continue;
            }
            b'*' => {
                index += 1;
                Token::Star
            }
            b',' => {
                index += 1;
                Token::Comma
            }
            b'(' => {
                index += 1;
                Token::LParen
            }
            b')' => {
                index += 1;
                Token::RParen
            }
            b';' => {
                index += 1;
                Token::Semicolon
            }
            b'=' => {
                index += 1;
                Token::Operator(Operator::Eq)
            }
            b'<' => {
                index += 1;
                let op = match bytes.get(index) {
                    Some(b'=') => Some(Operator::Le),
                    Some(b'>') => Some(Operator::Ne),
                    _ => None,
                };
                if let Some(op) = op {
                    index += 1;
                    Token::Operator(op)
                } else {
                    Token::Operator(Operator::Lt)
                }
            }
            b'>' => {
                index += 1;
                if bytes.get(index) == Some(&b'=') {
                    index += 1;
                    Token::Operator(Operator::Ge)
                } else {
                    Token::Operator(Operator::Gt)
                }
            }
            b'!' if bytes.get(index + 1) == Some(&b'=') => {
                index += 2;
                Token::Operator(Operator::Ne)
            }
            b'0'..=b'9' | b'-' => {
                let start = index;
                if byte == b'-' {
                    index += 1;
                    if !matches!(bytes.get(index), Some(b'0'..=b'9')) {
                        return Err(ParseError("expected digits after '-'".into()));
                    }
                }
                while matches!(bytes.get(index), Some(b'0'..=b'9')) {
                    index += 1;
                }
                if bytes.get(index) == Some(&b'.') {
                    index += 1;
                    if !matches!(bytes.get(index), Some(b'0'..=b'9')) {
                        return Err(ParseError("expected digits after decimal point".into()));
                    }
                    while matches!(bytes.get(index), Some(b'0'..=b'9')) {
                        index += 1;
                    }
                }
                Token::Number(sql[start..index].into())
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = index;
                index += 1;
                while matches!(
                    bytes.get(index),
                    Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')
                ) {
                    index += 1;
                }
                Token::Word(sql[start..index].into())
            }
            _ => {
                return Err(ParseError(format!(
                    "unsupported SQL character at byte {index}"
                )));
            }
        };
        tokens.push(token);
    }
    Ok(tokens)
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
                distinct: false,
                source: None,
                predicates: Vec::new(),
                or_predicates: Vec::new(),
                order_by: None,
                limit: None,
                offset: None,
            }
        );
        assert_eq!(
            parse_select("select id, price from part limit 2;").unwrap(),
            Select {
                projection: Projection::Columns(vec!["id".into(), "price".into()]),
                distinct: false,
                source: Some(Source::Table("part".into())),
                predicates: Vec::new(),
                or_predicates: Vec::new(),
                order_by: None,
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
    fn parses_numeric_aggregates_and_rejects_unsupported_functions() {
        for (name, expected) in [
            ("SUM", AggregateFunction::Sum),
            ("AVG", AggregateFunction::Avg),
            ("MIN", AggregateFunction::Min),
            ("MAX", AggregateFunction::Max),
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
            "SELECT COUNT() FROM part",
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
            "SELECT number FROM numbers(5) WHERE number NOT BETWEEN 1 AND 3",
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
            "SELECT number FROM numbers(5) WHERE number NOT IN (1)",
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
    fn bounds_sql_input_and_rejects_invalid_characters() {
        assert!(parse_select(&" ".repeat(4097)).is_err());
        assert!(parse_select("SELECT café FROM part").is_err());
    }
}
