use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Projection {
    Literal(i64),
    Count,
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
pub struct Predicate {
    pub column: String,
    pub operator: Operator,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Select {
    pub projection: Projection,
    pub source: Option<String>,
    pub predicate: Option<Predicate>,
    pub limit: Option<usize>,
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

    let projection = if parser.take(Token::Star) {
        Projection::All
    } else if parser.keyword("COUNT") {
        if !parser.take(Token::LParen) || !parser.take(Token::Star) || !parser.take(Token::RParen) {
            return Err(ParseError("expected COUNT(*)".into()));
        }
        Projection::Count
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

    let source = parser.keyword("FROM").then(|| parser.word()).transpose()?;
    if source.is_none() && !matches!(projection, Projection::Literal(_)) {
        return Err(ParseError("a table source is required".into()));
    }
    if source.is_some() && matches!(projection, Projection::Literal(_)) {
        return Err(ParseError("literal SELECT does not support FROM".into()));
    }

    let predicate = if parser.keyword("WHERE") {
        if source.is_none() {
            return Err(ParseError("WHERE requires a table source".into()));
        }
        Some(Predicate {
            column: parser.word()?,
            operator: parser.operator()?,
            value: parser.number()?,
        })
    } else {
        None
    };

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

    parser.take(Token::Semicolon);
    if parser.index != parser.tokens.len() {
        return Err(ParseError(
            "unexpected tokens after SELECT statement".into(),
        ));
    }
    Ok(Select {
        projection,
        source,
        predicate,
        limit,
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
                source: None,
                predicate: None,
                limit: None,
            }
        );
        assert_eq!(
            parse_select("select id, price from part limit 2;").unwrap(),
            Select {
                projection: Projection::Columns(vec!["id".into(), "price".into()]),
                source: Some("part".into()),
                predicate: None,
                limit: Some(2),
            }
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
                query.predicate.unwrap(),
                Predicate {
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
            "SELECT id FROM part WHERE id BETWEEN 1 AND 2",
            "SELECT id FROM part WHERE id > 'abc'",
            "SELECT id FROM part WHERE id =",
            "SELECT id FROM part LIMIT 184467440737095516161",
            "SELECT 1 + 2",
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
