use crate::lexer::{Token, tokenize};
use crate::{ParseError, Parser};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalarExpr {
    Column(String),
    TextLength(String),
    Integer(i64),
    Add(Box<ScalarExpr>, Box<ScalarExpr>),
    Subtract(Box<ScalarExpr>, Box<ScalarExpr>),
    Multiply(Box<ScalarExpr>, Box<ScalarExpr>),
    Divide(Box<ScalarExpr>, Box<ScalarExpr>),
}

impl ScalarExpr {
    pub fn referenced_columns(&self, out: &mut Vec<String>) {
        match self {
            Self::Column(name) => {
                if !out.contains(name) {
                    out.push(name.clone());
                }
            }
            Self::TextLength(name) => {
                if !out.contains(name) {
                    out.push(name.clone());
                }
            }
            Self::Integer(_) => {}
            Self::Add(left, right)
            | Self::Subtract(left, right)
            | Self::Multiply(left, right)
            | Self::Divide(left, right) => {
            {
                left.referenced_columns(out);
                right.referenced_columns(out);
            }
        }
    }
}

fn primary(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    match parser.tokens.get(parser.index) {
        Some(Token::Plus) => {
            parser.index += 1;
            primary(parser)
        }
        Some(Token::Minus) => {
            parser.index += 1;
            Ok(ScalarExpr::Subtract(
                Box::new(ScalarExpr::Integer(0)),
                Box::new(primary(parser)?),
            ))
        }
        Some(Token::Word(name))
            if name.eq_ignore_ascii_case("length")
                && parser.tokens.get(parser.index + 1) == Some(&Token::LParen) =>
        {
            parser.index += 2;
            let column = parser.word()?;
            if !parser.take(Token::RParen) {
                return Err(ParseError("length requires a single column".into()));
            }
            Ok(ScalarExpr::TextLength(column))
        }
        Some(Token::Word(_)) => Ok(ScalarExpr::Column(parser.word()?)),
        Some(Token::Number(number)) => {
            let number = number.parse::<i64>().map_err(|_| {
                ParseError("integer expression literal is out of Int64 range".into())
            })?;
            parser.index += 1;
            Ok(ScalarExpr::Integer(number))
        }
        Some(Token::LParen) => {
            parser.index += 1;
            let result = parse_expression_tokens(parser)?;
            if !parser.take(Token::RParen) {
                return Err(ParseError("expected ')' in scalar expression".into()));
            }
            Ok(result)
        }
        _ => Err(ParseError(
            "expected column, integer, or '(' in expression".into(),
        )),
    }
}

fn product(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    let mut expr = primary(parser)?;
    loop {
        if parser.take(Token::Star) {
            expr = ScalarExpr::Multiply(Box::new(expr), Box::new(primary(parser)?));
        } else if parser.take(Token::Slash) {
            expr = ScalarExpr::Divide(Box::new(expr), Box::new(primary(parser)?));
        } else {
            break;
        }
    }
    Ok(expr)
}

pub(crate) fn parse_expression_tokens(parser: &mut Parser) -> Result<ScalarExpr, ParseError> {
    let mut expr = product(parser)?;
    loop {
        let addition = if parser.take(Token::Plus) {
            true
        } else if parser.take(Token::Minus) {
            false
        } else {
            break;
        };
        let right = product(parser)?;
        expr = if addition {
            ScalarExpr::Add(Box::new(expr), Box::new(right))
        } else {
            ScalarExpr::Subtract(Box::new(expr), Box::new(right))
        };
    }
    Ok(expr)
}

pub fn parse_scalar_expression(sql: &str) -> Result<ScalarExpr, ParseError> {
    let mut parser = Parser {
        tokens: tokenize(sql)?,
        index: 0,
    };
    let expr = parse_expression_tokens(&mut parser)?;
    if parser.index != parser.tokens.len() {
        return Err(ParseError(
            "unexpected tokens after scalar expression".into(),
        ));
    }
    Ok(expr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_column_addition_and_subtraction() {
        let expr = parse_scalar_expression("ResolutionWidth + 89").unwrap();
        assert_eq!(
            expr,
            ScalarExpr::Add(
                Box::new(ScalarExpr::Column("ResolutionWidth".into())),
                Box::new(ScalarExpr::Integer(89)),
            )
        );
        assert!(matches!(
            parse_scalar_expression("ClientIP - 3").unwrap(),
            ScalarExpr::Subtract(..)
        ));
    }

    #[test]
    fn parses_nested_expressions_and_collects_unique_columns() {
        let expr = parse_scalar_expression("(width + 2) - width").unwrap();
        let mut columns = Vec::new();
        expr.referenced_columns(&mut columns);
        assert_eq!(columns, ["width"]);
    }

    #[test]
    fn accepts_string_length_inside_numeric_expressions() {
        let expr = parse_scalar_expression("length(URL) + 1").unwrap();
        let mut columns = Vec::new();
        expr.referenced_columns(&mut columns);
        assert_eq!(columns, ["URL"]);
        assert!(matches!(expr, ScalarExpr::Add(..)));
        assert!(parse_scalar_expression("length(URL, Other)").is_err());
    }

    #[test]
    fn multiplication_precedes_addition() {
        assert_eq!(
            parse_scalar_expression("width + 2 * 3").unwrap(),
            ScalarExpr::Add(
                Box::new(ScalarExpr::Column("width".into())),
                Box::new(ScalarExpr::Multiply(
                    Box::new(ScalarExpr::Integer(2)),
                    Box::new(ScalarExpr::Integer(3)),
                )),
            )
        );
        assert!(matches!(
            parse_scalar_expression("(width + 2) * 3").unwrap(),
            ScalarExpr::Multiply(..)
        ));
    }

    #[test]
    fn parses_unary_signs_on_columns_and_parentheses() {
        assert_eq!(
            parse_scalar_expression("-amount").unwrap(),
            ScalarExpr::Subtract(
                Box::new(ScalarExpr::Integer(0)),
                Box::new(ScalarExpr::Column("amount".into())),
            )
        );
        assert_eq!(
            parse_scalar_expression("+(amount * 2)").unwrap(),
            ScalarExpr::Multiply(
                Box::new(ScalarExpr::Column("amount".into())),
                Box::new(ScalarExpr::Integer(2)),
            )
        );
        assert!(parse_scalar_expression("-").is_err());
    }

    #[test]
    fn parses_division_with_multiplication_precedence() {
        assert_eq!(
            parse_scalar_expression("total / count + 1").unwrap(),
            ScalarExpr::Add(
                Box::new(ScalarExpr::Divide(
                    Box::new(ScalarExpr::Column("total".into())),
                    Box::new(ScalarExpr::Column("count".into())),
                )),
                Box::new(ScalarExpr::Integer(1)),
            )
        );
        assert!(parse_scalar_expression("total /").is_err());
    }

    #[test]
    fn rejects_invalid_expressions() {
        for value in ["width +", "width,", "(width + 1", "9223372036854775808"] {
            assert!(parse_scalar_expression(value).is_err(), "{value}");
        }
    }
}
